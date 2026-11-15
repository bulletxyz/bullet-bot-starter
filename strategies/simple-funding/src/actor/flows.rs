use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use bb_strategy_sdk::{OrderBook, Position, Side};
use rust_decimal::Decimal;

use super::{Day, LossStop, SimpleFundingActor, position_mark, usd};
use crate::adapter::io::{FlowError, Venue};
use crate::decide::decide;
use crate::limits::{FLATTEN_SLIPPAGE_MULT, LIQ_RATIO_FLATTEN};
use crate::plan::{Row, plan};

impl SimpleFundingActor {
    /// Margin: at the flatten ratio, flatten and halt; unknown, no new risk (a held position is
    /// never trimmed on a missing number). `true` when the ratio is known.
    async fn check_margin(&mut self, b: &Venue) -> bool {
        match b.margin_ratio().await {
            Ok(Some(ratio)) if ratio >= LIQ_RATIO_FLATTEN => {
                if !self.halted() {
                    tracing::error!(
                        %ratio, limit = %LIQ_RATIO_FLATTEN,
                        "simple-funding: margin ratio at the flatten line, flattening and halting"
                    );
                }
                self.halt();
                true
            }
            Ok(Some(_)) => true,
            Ok(None) => {
                tracing::warn!("simple-funding: margin ratio unreported, no entries this tick");
                false
            }
            Err(e) => {
                tracing::warn!(error = %e, "simple-funding: margin ratio unreadable, no entries this tick");
                false
            }
        }
    }

    /// A live start must not adopt positions it did not open: `decide` would keep or exit them
    /// like its own, and a shutdown would flatten them. Refused unless `adopt_existing_positions`.
    async fn refuse_foreign_positions(&mut self, b: &Venue) -> Result<(), FlowError> {
        if self.config.adopt_existing_positions {
            return Ok(());
        }
        let positions = b.get_positions().await.map_err(|e| {
            FlowError::strategy(format!("simple-funding: cannot check for existing positions: {e}"))
        })?;
        let foreign: Vec<&str> = positions
            .iter()
            .filter(|p| p.size > Decimal::ZERO && self.in_universe(&p.symbol))
            .filter(|p| !self.owned.contains(&p.symbol))
            .map(|p| p.symbol.as_str())
            .collect();
        if foreign.is_empty() {
            return Ok(());
        }
        self.refused = true;
        Err(FlowError::config(format!(
            "simple-funding: live start refused: the account already holds positions in the \
             universe that this bot did not open ({}). Close them, or set \
             adopt_existing_positions = true to let the bot manage and eventually flatten them",
            foreign.join(", ")
        )))
    }

    // -- tick -------------------------------------------------------------------------

    /// The universe's signed base positions from `positions`; keeps `held`, the entry costs
    /// and the owned symbols in step with them.
    fn track_holdings(&mut self, positions: &[Position]) -> HashMap<String, Decimal> {
        let held_base: HashMap<String, Decimal> = positions
            .iter()
            .filter(|p| self.in_universe(&p.symbol))
            .map(|p| {
                let signed = match p.side {
                    Some(Side::Buy) => p.size,
                    Some(Side::Sell) => -p.size,
                    None => Decimal::ZERO,
                };
                (p.symbol.clone(), signed)
            })
            .filter(|(_, v)| !v.is_zero())
            .collect();
        self.held = held_base.iter().map(|(k, v)| (k.clone(), *v)).collect();
        for (symbol, st) in &mut self.syms {
            if !held_base.contains_key(symbol) {
                st.entry_cost = None;
            }
        }
        let before = self.owned.len();
        self.owned.retain(|s| held_base.contains_key(s));
        if self.owned.len() != before {
            self.persist();
        }
        held_base
    }

    async fn on_tick(&mut self, cx: &Venue) -> Result<(), FlowError> {
        let b = cx;
        let now = cx.now();
        self.now = Some(now);
        let unix_ms = cx.unix_ms();

        // The venue is the truth; a failed read trades nothing.
        let positions = match b.get_positions().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "simple-funding: positions unreadable, no trading this tick");
                return Ok(());
            }
        };
        let equity = match b.get_account_equity().await {
            Ok(e) => e.map(|e| e.equity),
            Err(e) => {
                tracing::warn!(error = %e, "simple-funding: equity unreadable, no entries this tick");
                None
            }
        };
        self.equity = equity;
        self.read_once |= equity.is_some();
        self.roll_day(unix_ms, equity);
        self.check_loss(equity);

        let margin_known = self.check_margin(b).await;

        let held_base = self.track_holdings(&positions);
        self.gross_usd = positions
            .iter()
            .filter(|p| self.in_universe(&p.symbol))
            .fold(Decimal::ZERO, |sum, p| sum.saturating_add(usd(p.size.abs(), self.valuation(p))));

        if self.halted() {
            let actions = self.flatten_actions(&positions);
            let fallback = self.fallbacks(&positions);
            if !actions.is_empty() {
                self.execute(b, &actions, Decimal::from(FLATTEN_SLIPPAGE_MULT), &fallback).await;
            }
            return Ok(());
        }

        // Settle ambiguous commits against what the venue shows, then retry what is still open.
        self.reconcile_pending(&positions);
        if self.service_unwinds(b).await {
            // The positions above no longer hold what the venue has: plan nothing from them.
            return Ok(());
        }

        // An outstanding unwind is the repair of a lopsided book; the watchdog waits for it.
        let exit = if self.unwinds.is_empty() { self.imbalance_exit(&positions) } else { None };
        if let Some(exit) = exit {
            let fallback = self.fallbacks(&positions);
            self.execute(b, &[exit], Decimal::from(FLATTEN_SLIPPAGE_MULT), &fallback).await;
            return Ok(());
        }

        let held_set: HashSet<String> = held_base.keys().cloned().collect();
        let entry_equity = if margin_known && !self.entries_blocked() { equity } else { None };
        self.refresh_fees(cx, b).await;
        self.refresh_signals(cx, b, &held_set).await;
        self.refresh_oi(cx, b, &held_set).await;

        let foreign_gross: Decimal = positions
            .iter()
            .filter(|p| !self.in_universe(&p.symbol))
            .fold(Decimal::ZERO, |sum, p| sum.saturating_add(usd(p.size.abs(), position_mark(p))));
        let marks: HashMap<String, Decimal> =
            positions.iter().map(|p| (p.symbol.clone(), position_mark(p))).collect();
        let view = self.market_view(now, entry_equity, &held_base, &marks, foreign_gross);
        let targets = decide(&view);

        let rows: Vec<Row> = self
            .syms
            .iter()
            .zip(&targets)
            .map(|((symbol, st), t)| {
                let held = held_base.get(symbol).copied().unwrap_or(Decimal::ZERO);
                let entry = marks.get(symbol).copied().unwrap_or(Decimal::ZERO);
                Row {
                    symbol: symbol.clone(),
                    mark: st.mark.filter(|m| *m > Decimal::ZERO).unwrap_or(entry),
                    held_base: held,
                    step: st.step,
                    oi_base: st.oi_base,
                    fresh: self.fresh(st, now),
                    target_usd: t.notional_usd,
                }
            })
            .collect();
        let actions = plan(&rows, entry_equity, foreign_gross, &self.caps());
        let fallback = self.fallbacks(&positions);
        if !actions.is_empty() {
            self.execute(b, &actions, Decimal::ONE, &fallback).await;
        }
        Ok(())
    }
}

impl SimpleFundingActor {
    /// The actor's start (`Actor::init`) .
    ///
    /// # Errors
    /// No tradable market, an unreadable day state, or a live start over positions it did not open.
    pub async fn start(&mut self, cx: &Venue) -> Result<(), FlowError> {
        let b = cx;
        let listed: HashSet<String> = b.available_symbols().into_iter().collect();
        let mut dropped = Vec::new();
        for symbol in &self.symbols {
            let step = b.step_size(symbol);
            let known = listed.is_empty() || listed.contains(symbol);
            match step.filter(|s| *s > Decimal::ZERO) {
                Some(step) if known => {
                    self.syms.entry(symbol.clone()).or_default().step = Some(step);
                }
                _ => dropped.push(symbol.clone()),
            }
        }
        if !dropped.is_empty() {
            tracing::warn!(?dropped, "simple-funding: markets Bullet can't size, skipped");
        }
        if self.syms.is_empty() {
            return Err(FlowError::config("simple-funding: no tradable markets"));
        }
        if self.persists {
            // An unreadable record is not a fresh day: the baseline and the halt are unknown.
            let saved = self.saved_day.take().unwrap_or(Ok(None)).map_err(|e| {
                FlowError::config(format!(
                    "simple-funding: day state unreadable, not starting: {e}"
                ))
            })?;
            if let Some(saved) = saved {
                tracing::info!(
                    day = saved.day, start_equity = %saved.start_equity, halted = saved.halted,
                    owned = saved.owned.len(), "simple-funding: day state restored"
                );
                self.day = Some(Day { index: saved.day, start_equity: saved.start_equity });
                self.controls.halted.store(saved.halted, Ordering::SeqCst);
                self.owned =
                    saved.owned.into_iter().filter(|s| self.syms.contains_key(s)).collect();
            }
        }
        self.refuse_foreign_positions(b).await?;
        // A live start over positions (a restart, a resume after a hold that could not flatten)
        // did not see them being built: no entries until a tick reads the book balanced.
        self.recovering = b.get_positions().await.map_or(true, |ps| {
            ps.iter().any(|p| p.size > Decimal::ZERO && self.syms.contains_key(&p.symbol))
        });
        tracing::info!(markets = self.syms.len(), "simple-funding: ready");
        Ok(())
    }

    /// The actor's wind-down (`Actor::wind_down`) : flatten everything it holds.
    ///
    /// # Errors
    /// Positions remain after the last attempt.
    pub async fn stop(&mut self, cx: &Venue) -> Result<(), FlowError> {
        if self.refused {
            tracing::warn!("simple-funding: start was refused, positions left alone");
            return Ok(());
        }
        tracing::info!("simple-funding: winding down, flattening");
        let result = self.flatten_all(cx, "wind down").await;
        if result.is_ok() {
            self.owned.clear();
            self.persist();
        }
        result
    }

    /// What `/status` shows.
    #[must_use]
    pub fn status(&self) -> serde_json::Value {
        let live: BTreeMap<&String, String> = self
            .syms
            .iter()
            .filter_map(|(k, s)| Some((k, s.live_funding?.round_dp(6).to_string())))
            .collect();
        let signals: BTreeMap<&String, String> = self
            .syms
            .iter()
            .filter_map(|(k, s)| Some((k, s.signal?.round_dp(6).to_string())))
            .collect();
        serde_json::json!({
            "strategy": "simple-funding",
            "halted": self.halted(),
            "own_loss_stop": self.loss_stop == LossStop::Own,
            "entries_blocked": self.entries_blocked(),
            "unwinds": self.unwinds.iter().map(|(k, u)| (k.clone(), format!("{} {}", u.qty, u.attempts))).collect::<BTreeMap<_, _>>(),
            "pending_commits": self.pending.len(),
            "markets": self.syms.len(),
            "equity": self.equity.map(|e| e.to_string()),
            "day_start_equity": self.day.map(|d| d.start_equity.to_string()),
            "positions": self.held.iter().map(|(k, v)| (k.clone(), v.to_string())).collect::<BTreeMap<_, _>>(),
            "signals": signals,
            "live_funding_8h": live,
        })
    }
}

impl SimpleFundingActor {
    /// A mark price (`EventHandler<MarkPriceUpdate>`) .
    pub fn on_mark(&mut self, symbol: &str, price: Decimal, funding: Option<Decimal>, cx: &Venue) {
        if let Some(st) = self.syms.get_mut(symbol) {
            if price > Decimal::ZERO {
                st.mark = Some(price);
                st.mark_at = Some(cx.now());
            }
            if funding.is_some() {
                st.live_funding = funding;
            }
        }
    }

    /// A book (`EventHandler<BookUpdate>`) .
    pub fn on_book(&mut self, symbol: &str, book: Arc<OrderBook>, cx: &Venue) {
        if let Some(st) = self.syms.get_mut(symbol) {
            st.book = Some(book);
            st.book_at = Some(cx.now());
        }
    }

    /// A tick (`EventHandler<Tick>`) , then the report.
    ///
    /// # Errors
    /// The venue is not registered.
    pub async fn tick(&mut self, cx: &Venue) -> Result<(), FlowError> {
        let done = self.on_tick(cx).await;
        self.publish_report();
        done
    }
}
