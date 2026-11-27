use bb_strategy_sdk::{CancelOrder, NewOrder, OrderBook, OrderResult, OrderType, Side};
use bb_strategy_sdk::{ExchangeIntent, Intent, StrategyError, View};
use bb_strategy_support::ioc::{MAX_IOC_BPS_FROM_MID, capped_ioc_price, two_sided_mid};
use bb_strategy_support::touch;
use rust_decimal::Decimal;

use crate::adapter::{ENTRY_SLIP_BPS, GridStrategy};
use crate::levels::LevelState;

impl GridStrategy {
    /// A `Place` for every `Pending` level, skipping sides at the cap; a post-only level that
    /// would cross goes at the touch. While paused, an opening level is paused instead. Each
    /// level keeps the client id its order carries, so the venue's answer finds it again.
    pub(in crate::adapter) fn place_pending_orders(&mut self, view: &View<'_>) -> Vec<Intent> {
        if self.halt.is_some() || self.plan.is_none() {
            return Vec::new();
        }
        let order_type = self.config.order_type;
        let order_size = self.order_size();
        let max_position = self.cap();
        let (long_cap, short_cap) =
            self.side_caps.map_or((max_position, max_position), |(l, s)| {
                (l.min(max_position), s.min(max_position))
            });
        let net_position = self.position(view);
        let closing = self.state.closing(net_position, order_size);
        let mut projected_long = net_position + self.state.active_remaining(Side::Buy, order_size);
        let mut projected_short =
            net_position - self.state.active_remaining(Side::Sell, order_size);

        let mut orders: Vec<NewOrder> = Vec::new();
        let mut skipped_exposure = 0usize;
        // The side a directional grid never holds (cap zero): skipping it is expected, not a warning.
        let (long_unheld, short_unheld) =
            self.side_caps.map_or((false, false), |(l, s)| (l.is_zero(), s.is_zero()));
        let mut skipped_unheld = 0usize;

        for (idx, closes) in closing.into_iter().enumerate() {
            let (side, level_price) = {
                let l = &self.state.levels[idx];
                // A level with a client id has its Place in flight: its answer is still to come.
                if l.state != LevelState::Pending || l.client_id.is_some() {
                    continue;
                }
                let Some(side) = l.side else { continue };
                (side, l.price)
            };
            if self.paused() && !closes {
                self.state.pause(idx);
                continue;
            }
            let mut price = level_price;
            if order_type == OrderType::PostOnly
                && let Some(book) = self.book(view)
                && book.would_cross(side, price)
            {
                // Repriced to the touch on its own side: still a maker order.
                let touch = match side {
                    Side::Buy => book.best_bid(),
                    Side::Sell => book.best_ask(),
                };
                match touch {
                    Some(t) if !book.would_cross(side, t.price) => price = t.price,
                    _ => continue,
                }
            }

            match side {
                Side::Buy if projected_long + order_size > long_cap => {
                    skipped_unheld += usize::from(long_unheld);
                    skipped_exposure += usize::from(!long_unheld);
                    continue;
                }
                Side::Buy => projected_long += order_size,
                Side::Sell if projected_short - order_size < -short_cap => {
                    skipped_unheld += usize::from(short_unheld);
                    skipped_exposure += usize::from(!short_unheld);
                    continue;
                }
                Side::Sell => projected_short -= order_size,
            }

            let cid = self.client_ids.issue();
            self.state.levels[idx].client_id = Some(cid.clone());
            orders.push(NewOrder {
                protect: None,
                symbol: self.symbol().to_string(),
                side,
                order_type,
                price,
                quantity: order_size,
                client_id: Some(cid),
                reduce_only: false,
            });
        }

        if skipped_exposure > 0 {
            tracing::warn!(
                skipped = skipped_exposure,
                net_position = %net_position,
                max_position = %max_position,
                "Skipping grid levels that would exceed max_position if resting orders filled"
            );
        }
        if skipped_unheld > 0 {
            tracing::debug!(
                skipped = skipped_unheld,
                net_position = %net_position,
                "Skipping grid levels past the position on the side a directional grid never holds"
            );
        }
        if !orders.is_empty() {
            tracing::info!(count = orders.len(), "Placing grid orders");
        }
        orders.into_iter().map(|o| Self::on_bullet(ExchangeIntent::Place(o))).collect()
    }

    /// The venue's answer to a `Place`: the level goes Active, or back to retry.
    pub(in crate::adapter) fn placed(
        &mut self,
        order: &NewOrder,
        result: &OrderResult,
    ) -> Vec<Intent> {
        if let Some(cid) = &self.entry
            && order.client_id.as_deref() == Some(cid.as_str())
        {
            if result.success {
                self.entry = None;
                return Vec::new();
            }
            let why = result.error.as_deref().unwrap_or("refused");
            return vec![Intent::Shutdown(format!(
                "The grid's starting {} of {} {} didn't go through ({why}); it stopped rather \
                 than run without it.",
                order.side,
                order.quantity,
                self.symbol()
            ))];
        }
        let cid = order.client_id.as_deref();
        let Some(level) =
            self.state.levels.iter_mut().find(|l| cid.is_some() && l.client_id.as_deref() == cid)
        else {
            // A level let go while its Place was in flight: nothing tracks the order, so it is
            // cancelled. A reduce-only close never has a level.
            if !result.success || order.reduce_only {
                return Vec::new();
            }
            return vec![Self::on_bullet(ExchangeIntent::Cancel(CancelOrder {
                symbol: self.symbol().to_string(),
                order_id: result.order_id.clone().unwrap_or_default(),
                client_id: order.client_id.clone(),
            }))];
        };
        if result.success {
            level.state = LevelState::Active;
            if let Some(ref oid) = result.order_id {
                level.order_id = Some(oid.clone());
            }
        } else {
            // The host logs each refusal once (rate-limited): not again here, once per retry.
            tracing::debug!(
                error = result.error.as_deref().unwrap_or("unknown"),
                side = ?level.side,
                price = %level.price,
                "Place failed — leaving level Pending for retry"
            );
            level.client_id = None;
        }
        Vec::new()
    }

    /// The starting position's IOC at the touch, capped around the mid.
    pub(in crate::adapter) fn entry_order(
        &mut self,
        (side, qty): (Side, Decimal),
        book: &OrderBook,
    ) -> Result<NewOrder, StrategyError> {
        let no_book = || {
            StrategyError(format!(
                "No two-sided book on {} to open the starting position",
                self.config.symbol
            ))
        };
        let mid = two_sided_mid(book).ok_or_else(no_book)?;
        let touch = touch(book, side).ok_or_else(no_book)?;
        let price = capped_ioc_price(side, touch, mid, ENTRY_SLIP_BPS, MAX_IOC_BPS_FROM_MID)
            .ok_or_else(no_book)?;
        // The hard stop on the losing side rides on the entry: the venue holds it from the fill.
        let stop = self.plan.as_ref().and_then(|p| match side {
            Side::Buy => p.stop_below,
            Side::Sell => p.stop_above,
        });
        let protect = stop.filter(|s| *s > Decimal::ZERO).map(bb_strategy_sdk::Protection::stop_at);
        let cid = self.client_ids.issue();
        self.entry = Some(cid.clone());
        Ok(NewOrder {
            protect,
            symbol: self.symbol().to_string(),
            side,
            order_type: OrderType::Market,
            price,
            quantity: qty,
            client_id: Some(cid),
            reduce_only: false,
        })
    }
}
