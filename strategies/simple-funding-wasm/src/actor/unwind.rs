//! Unwinds of one-sided legs, ambiguous commits, the balance watchdog and the flatten list.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal USD notionals and sizes sit far below 7.9e28; the backoff shift is capped"
)]

use std::collections::BTreeMap;

use bb_strategy_sdk::{Decimal, Position, Side};

use super::{Core, Pending, PendingLeg, Unwind, grown, position_mark, usd};
use crate::limits::{
    AMBIGUOUS_SETTLE_SECS, FLATTEN_SLIPPAGE_MULT, IMBALANCE_TICKS, UNWIND_BACKOFF_BASE_SECS,
    UNWIND_BACKOFF_MAX_SECS,
};
use crate::plan::{Action, floor_to_step, opposite};

impl Core {
    pub(crate) fn hold_pending(&mut self, legs: Vec<PendingLeg>) {
        let since_ms = self.now_ms();
        self.pending.push(Pending { legs, since_ms });
    }

    /// Record that `qty` of the leg this strategy opened must be closed, merging with an unwind
    /// already outstanding on the symbol.
    fn queue_unwind(&mut self, symbol: &str, side: Side, qty: Decimal) {
        let now = self.now_ms();
        let entry = self.unwinds.entry(symbol.to_owned()).or_insert(Unwind {
            side,
            qty: Decimal::ZERO,
            attempts: 0,
            next_at_ms: now,
        });
        entry.qty = entry.qty.max(qty);
    }

    /// Close `qty` of what `leg` just opened, crossing wide. Failure (rejected, an IOC that
    /// matched nothing, an error) leaves it outstanding: retried with backoff, and no new pair
    /// opens meanwhile.
    pub(crate) async fn unwind(&mut self, leg: &Action, qty: Decimal) {
        if qty <= Decimal::ZERO {
            return;
        }
        self.queue_unwind(&leg.symbol, leg.side, qty);
        self.try_unwind(&leg.symbol).await;
    }

    /// One unwind attempt on the venue's truth: what is still open in the leg's direction is
    /// closed reduce-only, then read back. Accepted is not closed.
    async fn try_unwind(&mut self, symbol: &str) {
        let Some(u) = self.unwinds.get(symbol).cloned() else { return };
        let now = self.now_ms();
        let step = self.syms.get(symbol).and_then(|s| s.step);
        let dust = |q: Decimal| q <= Decimal::ZERO || step.is_some_and(|s| q < s);
        let open_in_leg = |signed: Decimal| grown(u.side, Decimal::ZERO, signed).max(Decimal::ZERO);
        let Some(signed) = self.signed_now(symbol).await else {
            self.unwind_failed(symbol, now);
            return;
        };
        let qty = u.qty.min(open_in_leg(signed));
        if dust(qty) {
            self.unwinds.remove(symbol);
            return;
        }
        let mult = Decimal::from(FLATTEN_SLIPPAGE_MULT * (u.attempts.min(4) + 1));
        let close =
            Action { symbol: symbol.to_owned(), side: opposite(u.side), qty, reduce_only: true };
        self.place_one(&close, mult, &BTreeMap::new()).await;
        match self.signed_now(symbol).await.map(open_in_leg) {
            Some(left) if dust(left) => {
                self.unwinds.remove(symbol);
            }
            left => {
                if let (Some(left), Some(u)) = (left, self.unwinds.get_mut(symbol)) {
                    u.qty = u.qty.min(left);
                }
                self.unwind_failed(symbol, now);
            }
        }
    }

    fn unwind_failed(&mut self, symbol: &str, now_ms: u64) {
        let Some(u) = self.unwinds.get_mut(symbol) else { return };
        u.attempts = u.attempts.saturating_add(1);
        let wait = UNWIND_BACKOFF_BASE_SECS
            .saturating_mul(1_u64 << u.attempts.saturating_sub(1).min(16))
            .min(UNWIND_BACKOFF_MAX_SECS);
        u.next_at_ms = now_ms.saturating_add(wait * 1_000);
    }

    /// Retry every unwind whose backoff is over. `true` when it placed or read anything: the
    /// positions this tick read are then out of date.
    pub(crate) async fn service_unwinds(&mut self) -> bool {
        let now = self.now_ms();
        let due: Vec<String> = self
            .unwinds
            .iter()
            .filter(|(_, u)| now >= u.next_at_ms)
            .map(|(symbol, _)| symbol.clone())
            .collect();
        let acted = !due.is_empty();
        for symbol in due {
            self.try_unwind(&symbol).await;
        }
        acted
    }

    /// Settle the legs whose submit was ambiguous, from the positions this tick read: confirmed
    /// legs stay (matched in USD), anything one-sided beyond the tolerance is queued for unwind.
    /// Only a group older than [`AMBIGUOUS_SETTLE_SECS`] is judged.
    pub(crate) fn reconcile_pending(&mut self, positions: &[Position]) {
        if self.pending.is_empty() {
            return;
        }
        let now = self.now_ms();
        let settle = AMBIGUOUS_SETTLE_SECS * 1_000;
        let (due, wait): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| now.saturating_sub(p.since_ms) >= settle);
        self.pending = wait;
        for group in due {
            let grew: Vec<(&PendingLeg, Decimal)> = group
                .legs
                .iter()
                .map(|l| {
                    let signed = bb_strategy_sdk::signed_position(positions, &l.symbol);
                    (l, grown(l.side, l.before, signed).max(Decimal::ZERO))
                })
                .collect();
            let matched = if grew.len() > 1 {
                grew.iter().map(|(l, q)| usd(*q, l.price)).min().unwrap_or(Decimal::ZERO)
            } else {
                Decimal::ZERO
            };
            for (leg, qty) in grew {
                let value = usd(qty, leg.price);
                if qty <= Decimal::ZERO || value - matched <= value * crate::limits::PAIR_TOLERANCE
                {
                    continue;
                }
                let raw = (value - matched).checked_div(leg.price).unwrap_or(qty).min(qty);
                let step = self.syms.get(&leg.symbol).and_then(|s| s.step);
                let excess = step.map_or(raw, |s| floor_to_step(raw, s));
                self.queue_unwind(&leg.symbol, leg.side, excess);
            }
        }
    }

    /// The price a position is valued at: the live mark, else the mark the venue's own unrealised
    /// `PnL` implies, else its entry price.
    pub(crate) fn valuation(&self, p: &Position) -> Decimal {
        self.io
            .with(|s| s.seen.get(&p.symbol).and_then(|m| m.mark))
            .filter(|m| *m > Decimal::ZERO)
            .unwrap_or_else(|| position_mark(p))
    }

    /// Tracks how long the long and short books have sat past `max_net_pct`; after
    /// [`IMBALANCE_TICKS`] ticks returns the reduce-only exit of the weakest leg (lowest absolute
    /// signal, unknown first) on the heavier side.
    pub(crate) fn imbalance_exit(&mut self, positions: &[Position]) -> Option<Action> {
        let mut legs: Vec<(Side, &Position, Decimal)> = Vec::new();
        let (mut long, mut short) = (Decimal::ZERO, Decimal::ZERO);
        for p in positions.iter().filter(|p| p.size > Decimal::ZERO && self.in_universe(&p.symbol))
        {
            let Some(side) = p.side else { continue };
            let value = usd(p.size, self.valuation(p));
            match side {
                Side::Buy => long = long.saturating_add(value),
                Side::Sell => short = short.saturating_add(value),
            }
            let rank =
                self.syms.get(&p.symbol).and_then(|s| s.signal).map_or(Decimal::ZERO, |s| s.abs());
            legs.push((side, p, rank));
        }
        let gross = long.saturating_add(short);
        let skew = if gross.is_zero() {
            Decimal::ZERO
        } else {
            (long - short).abs().checked_div(gross).unwrap_or(Decimal::ONE)
        };
        if skew <= self.config.max_net_pct {
            self.imbalance_ticks = 0;
            self.recovering = false;
            return None;
        }
        self.imbalance_ticks = self.imbalance_ticks.saturating_add(1);
        // A start over a lopsided book (an orphan left by a crash) sheds at once.
        let grace = if self.recovering { 1 } else { IMBALANCE_TICKS };
        if self.imbalance_ticks < grace {
            return None;
        }
        self.imbalance_ticks = 0;
        let heavy = if long > short { Side::Buy } else { Side::Sell };
        let (_, weakest, _) = legs
            .into_iter()
            .filter(|(side, _, _)| *side == heavy)
            .min_by(|a, b| a.2.cmp(&b.2).then_with(|| a.1.symbol.cmp(&b.1.symbol)))?;
        Some(Action {
            symbol: weakest.symbol.clone(),
            side: opposite(heavy),
            qty: weakest.size,
            reduce_only: true,
        })
    }

    /// Reduce-only orders closing every universe position in `positions`.
    pub(crate) fn flatten_actions(&self, positions: &[Position]) -> Vec<Action> {
        positions
            .iter()
            .filter(|p| p.size > Decimal::ZERO && self.in_universe(&p.symbol))
            .filter_map(|p| {
                Some(Action {
                    symbol: p.symbol.clone(),
                    side: opposite(p.side?),
                    qty: p.size,
                    reduce_only: true,
                })
            })
            .collect()
    }

    pub(crate) fn fallbacks(&self, positions: &[Position]) -> BTreeMap<String, Decimal> {
        positions.iter().map(|p| (p.symbol.clone(), self.valuation(p))).collect()
    }
}
