use bb_strategy_sdk::ValidateConfig;
use bb_strategy_sdk::{
    BULLET, Event, ExchangeIntent, HistoryNeed, Intent, MarketEvent, OrderResult,
};
use bb_strategy_sdk::{StrategyAdapter, StrategyError, View, Warmup};
use rust_decimal::Decimal;

use crate::adapter::{GridStrategy, KEY_HARD_STOP, Seen};
use crate::derive::Direction;
use crate::plan::{self, HardLimits, Market, Plan};
use crate::report::{GridPnl, GridReport};

impl StrategyAdapter for GridStrategy {
    type Report = GridReport;

    /// The hard stop past the range on the position's losing side: the venue holds it while the bot
    /// is down.
    fn protective_stop(&self, view: &View<'_>) -> Option<bb_strategy_sdk::Protection> {
        let plan = self.plan.as_ref()?;
        let stop = match self.position(view).cmp(&Decimal::ZERO) {
            std::cmp::Ordering::Greater => plan.stop_below,
            std::cmp::Ordering::Less => plan.stop_above,
            std::cmp::Ordering::Equal => None,
        };
        stop.filter(|s| *s > Decimal::ZERO).map(bb_strategy_sdk::Protection::stop_at)
    }

    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        self.config.validate().map_err(StrategyError)?;
        // A start that takes over what a kept shutdown left holds that position as its own.
        let Some(shown) = self.market(view) else {
            return Err(StrategyError(format!(
                "The host shows no {} market on {BULLET}: the grid trades only that one.",
                self.symbol()
            )));
        };
        if !shown.position.is_zero() && !self.reclaiming {
            return Err(StrategyError(format!(
                "The account already holds {} {}: the grid starts from no position. Close it on \
                 the venue (or use another sub-account), then start the grid.",
                shown.position,
                self.symbol()
            )));
        }
        let book = shown.book().ok_or_else(|| {
            StrategyError("No orderbook data available to compute mid at startup".into())
        })?;
        let mid = book.midpoint().ok_or_else(|| {
            StrategyError("No orderbook data available to compute mid at startup".into())
        })?;
        if let Some(why) = &self.history_gap {
            return Err(StrategyError(format!(
                "The grid can't set its range for {}: {why}",
                self.symbol()
            )));
        }
        self.tick = shown.facts.tick_size;
        let market = Market { mid, tick: shown.facts.tick_size, lot: shown.facts.step_size };
        // The host's limits in force, when it shows them, else the build's.
        let limits = view.limits.as_ref().map_or(self.limits, |l| HardLimits {
            max_position: Some(l.max_position),
            max_notional_usd: l.max_notional_usd,
        });
        let plan = plan::resolve(&self.config, market, self.history.as_ref(), limits)
            .map_err(StrategyError)?;
        self.check_fee_floor(&plan)?;
        self.state.compute_levels(&plan, self.tick);
        tracing::info!(
            lower = %plan.lower,
            upper = %plan.upper,
            levels = plan.levels,
            spacing = %plan.spacing(),
            order_size = %plan.order_size,
            anchor = %plan.anchor,
            direction = ?plan.direction,
            stop_below = ?plan.stop_below.map(|p| p.to_string()),
            stop_above = ?plan.stop_above.map(|p| p.to_string()),
            worst_notional = %plan.worst_notional,
            mid = %mid,
            "Grid initialized"
        );
        let entry = plan.entry;
        let order_size = plan.order_size;
        let directional = plan.direction != Direction::Neutral;
        self.plan = Some(plan);
        self.see(view);
        // A directional grid never holds the side no hard stop guards; a reclaimed grid is also held
        // to its plan's worst case on each side, with the kept position counted.
        if directional || self.reclaiming {
            self.side_caps = self.worst_by_side(order_size);
        }
        // A kept run's position: taken over, and the grid goes on from it.
        let resumed = self.reclaiming && !shown.position.is_zero();
        if resumed && let Some(halted) = self.check_halt(view, false) {
            // Past its hard stop already: it stops and closes now, as it would have running.
            return Ok(halted);
        }

        // The trend filter before the first placement: a market already trending at start holds
        // the opening orders back before they go out.
        let mut intents = self.handle_trend_filter(view);
        if let Some(entry) = entry.filter(|_| !resumed) {
            intents.push(Self::on_bullet(ExchangeIntent::Place(self.entry_order(entry, book)?)));
        }
        intents.extend(self.place_pending_orders(view));
        // A grid started again (the owner resumed the hold) ends the last hard stop's alert.
        if self.hard_stop_hold.is_some() {
            let resolve = bb_strategy_sdk::Alert::Resolve { key: KEY_HARD_STOP.to_string() };
            intents.push(Intent::Notify(resolve));
        }
        Ok(intents)
    }

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent> {
        // A cancel's answer counts in any state: the stop's (and the wind-down's) cancel may fail.
        match event {
            Event::Outcome {
                intent:
                    Intent::On {
                        intent: ExchangeIntent::CancelAll(_) | ExchangeIntent::CancelOwn(_),
                        ..
                    },
                result,
            } => {
                let why = result.error.as_deref().unwrap_or("refused");
                return self.pull.answered((!result.success).then_some(why)).into_iter().collect();
            }
            Event::Failed {
                intent:
                    Intent::On {
                        intent: ExchangeIntent::CancelAll(_) | ExchangeIntent::CancelOwn(_),
                        ..
                    },
                error,
            } => {
                return self.pull.answered(Some(error)).into_iter().collect();
            }
            _ => {}
        }
        // Nothing to do before the start, or without the grid's market in the view.
        if self.plan.is_none() || self.market(view).is_none() {
            return Vec::new();
        }
        self.see(view);
        match event {
            Event::Market { exchange, symbol, event }
                if exchange == BULLET && symbol == self.symbol() =>
            {
                match event {
                    MarketEvent::Fill(trade) => self.on_fill(trade, view),
                    MarketEvent::Order(order) => {
                        self.on_order(order);
                        Vec::new()
                    }
                    MarketEvent::Book => self.on_book(view),
                    MarketEvent::Mark(_) => Vec::new(),
                }
            }
            Event::Tick => self.on_tick(view),
            // An acked place holds its level as an unknown one does: a read may lag the ack too.
            Event::Outcome {
                intent: Intent::On { intent: ExchangeIntent::Place(order), .. },
                result,
            } => {
                if result.success
                    && let Some(c) = &order.client_id
                {
                    self.placed_at.insert(c.clone(), view.now_ms);
                }
                self.placed(order, result)
            }
            // A place the host never sent (one that may have landed comes as `Unknown`) is refused.
            Event::Failed {
                intent: Intent::On { intent: ExchangeIntent::Place(order), .. },
                error,
            } => {
                let refused = OrderResult {
                    order_id: None,
                    client_id: order.client_id.clone(),
                    success: false,
                    error: Some(error.clone()),
                };
                self.placed(order, &refused)
            }
            // A place that may have landed holds its level as placed (order id unknown): it is not
            // placed again. A later read that does not list it frees the level (reconcile).
            Event::Unknown {
                intent: Intent::On { intent: ExchangeIntent::Place(order), .. },
                ..
            } => {
                let held = OrderResult {
                    order_id: None,
                    client_id: order.client_id.clone(),
                    success: true,
                    error: None,
                };
                if let Some(c) = &order.client_id {
                    self.placed_at.insert(c.clone(), view.now_ms);
                }
                self.placed(order, &held)
            }
            // Other outcomes, and any event a later bb-strategy adds: nothing to do.
            _ => Vec::new(),
        }
    }

    fn stop(&mut self, view: &View<'_>) -> Vec<Intent> {
        // A stopped grid with a position left closes it on the way out; otherwise cancel only (a
        // reconfigure's flatten is the supervisor's).
        tracing::info!("Shutting down grid — cancelling all orders");
        let mut out = vec![self.cancel_all()];
        if let Some(attempt) = self.halt.as_ref().map(|h| h.attempts.saturating_add(1)) {
            out.extend(self.close(view, attempt));
        }
        out
    }

    fn report(&self) -> GridReport {
        let Seen { mid, position, avg_entry, realized } = self.seen.clone();
        let unrealized = match (mid, avg_entry) {
            (Some(m), Some(e)) => (m - e) * position,
            _ => Decimal::ZERO,
        };
        let usd = |d: Decimal| d.round_dp(2).normalize();
        let plan = self.plan.as_ref();
        GridReport {
            direction: plan.map(|p| format!("{:?}", p.direction).to_lowercase()),
            range: format!("{:?}", self.config.range).to_lowercase(),
            lower_price: plan.map(|p| p.lower),
            upper_price: plan.map(|p| p.upper),
            anchor: plan.map(|p| p.anchor),
            spacing: plan.map(Plan::spacing),
            order_size: plan.map(|p| p.order_size),
            stop_below: plan.and_then(|p| p.stop_below),
            stop_above: plan.and_then(|p| p.stop_above),
            worst_case_usd: plan.map(|p| usd(p.worst_notional)),
            amount_usd: plan.map(|p| p.dollars),
            active_levels: self.state.active_count(),
            total_levels: self.state.levels.len(),
            paused: self.paused(),
            stopped: self.halt.as_ref().map(|h| h.reason.clone()),
            pnl: GridPnl {
                realized: usd(realized),
                unrealized: usd(unrealized),
                total: usd(realized + unrealized),
            },
            levels: self.state.levels.clone(),
            history: self.warmup_note.clone(),
        }
    }

    fn history(&self) -> Option<HistoryNeed> {
        self.history_need()
    }

    fn histories(&self) -> Vec<HistoryNeed> {
        self.history_needs()
    }

    fn warm_up_for(&mut self, need: HistoryNeed, warmup: Warmup) {
        self.warm_for(need, warmup);
    }

    fn reclaim(&mut self) {
        self.reclaiming = true;
    }

    /// A stopped grid (its hard stop, the daily loss stop) closes at a kept shutdown too.
    fn may_keep(&self) -> bool {
        self.halt.is_none()
    }

    fn warm_up(&mut self, warmup: Warmup) {
        self.warm(warmup);
    }
}
