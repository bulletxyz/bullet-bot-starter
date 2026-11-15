//! One tick: read the venue, run the stops, refresh the data, decide, plan, trade.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal USD notionals and sizes sit far below 7.9e28"
)]

use std::collections::{BTreeMap, BTreeSet};

use bb_strategy_sdk::{Decimal, Position, Side};

use super::{Core, DAY_MS, Day, position_mark, usd};
use crate::cost::{assess, fillable_within};
use crate::decide::{MarketView, SymbolView, decide};
use crate::limits::{FLATTEN_SLIPPAGE_MULT, LIQ_RATIO_FLATTEN};
use crate::plan::{Row, plan};

impl Core {
    /// Track the UTC day and the halt: a new day lifts it and restarts the allowance from the
    /// current equity.
    fn roll_day(&mut self, unix_ms: u64, equity: Option<Decimal>) {
        let index = unix_ms / DAY_MS;
        let Some(equity) = equity else { return };
        // Ordered, never merely different: a clock that steps back is not a new day.
        if self.day.is_none_or(|d| index > d.index) {
            self.halted = false;
            self.day = Some(Day { index, start_equity: equity });
        }
    }

    /// Halt when equity has dropped `daily_loss_usd` since the day start.
    fn check_loss(&mut self, equity: Option<Decimal>) {
        let (Some(day), Some(equity)) = (self.day, equity) else { return };
        if !self.halted && day.start_equity - equity >= self.config.loss_stop_usd() {
            self.halted = true;
        }
    }

    /// Margin: at the flatten ratio, halt; unknown, no new risk (a held position is never trimmed
    /// on a missing number). `true` when the ratio is known.
    async fn check_margin(&mut self) -> bool {
        match self.margin_ratio().await {
            Ok(Some(ratio)) if ratio >= LIQ_RATIO_FLATTEN => {
                self.halted = true;
                true
            }
            Ok(Some(_)) => true,
            Ok(None) | Err(_) => false,
        }
    }

    /// The universe's signed base positions from `positions`; keeps `held`, the entry costs and
    /// the owned symbols in step with them.
    fn track_holdings(&mut self, positions: &[Position]) -> BTreeMap<String, Decimal> {
        let held_base: BTreeMap<String, Decimal> = positions
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
        self.held.clone_from(&held_base);
        for (symbol, st) in &mut self.syms {
            if !held_base.contains_key(symbol) {
                st.entry_cost = None;
            }
        }
        self.owned.retain(|s| held_base.contains_key(s));
        held_base
    }

    fn market_view(
        &self,
        now_ms: u64,
        equity: Option<Decimal>,
        held_base: &BTreeMap<String, Decimal>,
        marks: &BTreeMap<String, Decimal>,
        foreign_gross_usd: Decimal,
    ) -> MarketView {
        let seen = self.io.with(|s| s.seen.clone());
        let symbols = self
            .syms
            .iter()
            .map(|(symbol, st)| {
                let m = seen.get(symbol);
                let mark = m.and_then(|m| m.mark).unwrap_or(Decimal::ZERO);
                let price = if mark > Decimal::ZERO {
                    mark
                } else {
                    marks.get(symbol).copied().unwrap_or(Decimal::ZERO)
                };
                let book = m.and_then(|m| m.book.as_ref());
                let cost = book.map(|b| assess(b, st.fee_bps, self.config.notional_per_symbol));
                // The walk the cost was judged on must fit inside the IOC cap, else every entry
                // fills part of its size: thin depth then reads as none.
                let fits = book.is_some_and(|b| {
                    fillable_within(b, self.config.notional_per_symbol, self.config.slippage_bps)
                });
                SymbolView {
                    symbol: symbol.clone(),
                    signal_8h: st.signal,
                    round_trip_cost: cost.and_then(|c| c.round_trip),
                    mark,
                    oi_usd: st.oi_base.and_then(|oi| oi.checked_mul(mark)),
                    depth_usd: cost.and_then(|c| c.depth_usd).filter(|_| fits),
                    fresh: self.fresh(symbol, now_ms),
                    held_usd: usd(held_base.get(symbol).copied().unwrap_or(Decimal::ZERO), price),
                    entry_cost: st.entry_cost,
                }
            })
            .collect();
        MarketView { symbols, equity, foreign_gross_usd, params: self.params() }
    }

    /// The first tick's check that the account holds no universe position this strategy did not
    /// open (unless `adopt_existing_positions`): `Some(why)` stops it trading.
    fn foreign_positions(&self, positions: &[Position]) -> Option<String> {
        if self.config.adopt_existing_positions {
            return None;
        }
        let foreign: Vec<&str> = positions
            .iter()
            .filter(|p| p.size > Decimal::ZERO && self.in_universe(&p.symbol))
            .filter(|p| !self.owned.contains(&p.symbol))
            .map(|p| p.symbol.as_str())
            .collect();
        (!foreign.is_empty()).then(|| {
            format!(
                "the account already holds positions in the universe that this strategy did not \
                 open ({}); close them, or set adopt_existing_positions to let it manage and \
                 flatten them",
                foreign.join(", ")
            )
        })
    }

    pub async fn on_tick(&mut self) {
        self.tick().await;
        self.publish();
    }

    async fn tick(&mut self) {
        if self.refused.is_some() {
            return;
        }
        let now = self.now_ms();
        // The venue is the truth; a failed read trades nothing.
        let Ok(positions) = self.positions().await else { return };
        if !self.checked_start {
            self.checked_start = true;
            if let Some(why) = self.foreign_positions(&positions) {
                self.refused = Some(why);
                return;
            }
            // A start over positions did not see them being built: no entries until a tick reads
            // the book balanced.
            self.recovering =
                positions.iter().any(|p| p.size > Decimal::ZERO && self.in_universe(&p.symbol));
        }
        let equity = self.equity().await.unwrap_or(None);
        self.equity = equity;
        self.roll_day(now, equity);
        self.check_loss(equity);
        let margin_known = self.check_margin().await;

        let held_base = self.track_holdings(&positions);
        self.gross_usd = positions
            .iter()
            .filter(|p| self.in_universe(&p.symbol))
            .fold(Decimal::ZERO, |sum, p| sum.saturating_add(usd(p.size.abs(), self.valuation(p))));

        if self.halted {
            let actions = self.flatten_actions(&positions);
            let fallback = self.fallbacks(&positions);
            if !actions.is_empty() {
                self.execute(&actions, Decimal::from(FLATTEN_SLIPPAGE_MULT), &fallback).await;
            }
            return;
        }

        // Settle ambiguous commits against what the venue shows, then retry what is still open.
        self.reconcile_pending(&positions);
        if self.service_unwinds().await {
            // The positions above no longer hold what the venue has: plan nothing from them.
            return;
        }

        // An outstanding unwind is the repair of a lopsided book; the watchdog waits for it.
        let exit = if self.unwinds.is_empty() { self.imbalance_exit(&positions) } else { None };
        if let Some(exit) = exit {
            let fallback = self.fallbacks(&positions);
            self.execute(&[exit], Decimal::from(FLATTEN_SLIPPAGE_MULT), &fallback).await;
            return;
        }

        let held_set: BTreeSet<String> = held_base.keys().cloned().collect();
        let stopped = self.io.with(|s| s.order_stop.is_some());
        let entry_equity =
            if margin_known && !self.entries_blocked() && !stopped { equity } else { None };
        self.refresh_fees().await;
        self.refresh_signals(&held_set).await;
        self.refresh_oi(&held_set).await;
        let now = self.now_ms();

        let foreign_gross: Decimal = positions
            .iter()
            .filter(|p| !self.in_universe(&p.symbol))
            .fold(Decimal::ZERO, |sum, p| sum.saturating_add(usd(p.size.abs(), position_mark(p))));
        let marks: BTreeMap<String, Decimal> =
            positions.iter().map(|p| (p.symbol.clone(), position_mark(p))).collect();
        let view = self.market_view(now, entry_equity, &held_base, &marks, foreign_gross);
        let targets = decide(&view);

        let seen = self.io.with(|s| s.seen.clone());
        let rows: Vec<Row> = self
            .syms
            .iter()
            .zip(&targets)
            .map(|((symbol, st), t)| {
                let entry = marks.get(symbol).copied().unwrap_or(Decimal::ZERO);
                Row {
                    symbol: symbol.clone(),
                    mark: seen
                        .get(symbol)
                        .and_then(|m| m.mark)
                        .filter(|m| *m > Decimal::ZERO)
                        .unwrap_or(entry),
                    held_base: held_base.get(symbol).copied().unwrap_or(Decimal::ZERO),
                    step: st.step,
                    oi_base: st.oi_base,
                    fresh: self.fresh(symbol, now),
                    target_usd: t.notional_usd,
                }
            })
            .collect();
        let actions = plan(&rows, entry_equity, foreign_gross, &self.caps());
        let fallback = self.fallbacks(&positions);
        if !actions.is_empty() {
            self.execute(&actions, Decimal::ONE, &fallback).await;
        }
    }
}
