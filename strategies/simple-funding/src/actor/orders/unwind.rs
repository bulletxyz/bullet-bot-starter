//! Legs being closed again, ambiguous commits, and the shed of a lopsided book.

use std::collections::HashMap;
use std::time::Duration;

use bb_strategy_sdk::{Position, Side, signed_position};
use rust_decimal::Decimal;

use crate::actor::{Pending, PendingLeg, SimpleFundingActor, Unwind, grown, usd};
use crate::adapter::io::{Mono, Venue};
use crate::limits::{
    AMBIGUOUS_SETTLE_SECS, FLATTEN_SLIPPAGE_MULT, IMBALANCE_TICKS, PAIR_TOLERANCE,
    UNWIND_BACKOFF_BASE_SECS, UNWIND_BACKOFF_MAX_SECS, UNWIND_ESCALATE_AFTER,
};
use crate::plan::{Action, floor_to_step, opposite};

impl SimpleFundingActor {
    // -- unwinds and ambiguous commits ----------------------------------------------------

    /// Entries wait while a leg's commit is unsettled, an unwind is outstanding, or the start
    /// is still recovering a book it found lopsided.
    pub(in crate::actor) fn entries_blocked(&self) -> bool {
        !self.unwinds.is_empty() || !self.pending.is_empty() || self.recovering
    }

    pub(super) fn hold_pending(&mut self, legs: Vec<PendingLeg>) {
        let Some(since) = self.now else { return };
        self.pending.push(Pending { legs, since });
    }

    fn unwind_key(symbol: &str) -> String {
        format!("simple_funding_unwind_{symbol}")
    }

    /// Record that `qty` of the `leg` this bot opened must be closed, merging with an unwind
    /// already outstanding on the symbol.
    fn queue_unwind(&mut self, symbol: &str, side: Side, qty: Decimal) {
        let Some(now) = self.now else { return };
        let entry = self.unwinds.entry(symbol.to_string()).or_insert(Unwind {
            side,
            qty: Decimal::ZERO,
            attempts: 0,
            next_at: now,
        });
        entry.qty = entry.qty.max(qty);
    }

    /// Close `qty` of what `leg` just opened, crossing wide. Failure (rejected, an IOC that
    /// matched nothing, an error) leaves it outstanding: retried with backoff, alerted, and no
    /// new pair opens meanwhile.
    pub(super) async fn unwind(&mut self, b: &Venue, leg: &Action, qty: Decimal) {
        if qty <= Decimal::ZERO {
            return;
        }
        tracing::error!(
            symbol = %leg.symbol, %qty,
            "simple-funding: the pair's other leg did not fill, unwinding this one"
        );
        self.queue_unwind(&leg.symbol, leg.side, qty);
        self.try_unwind(b, &leg.symbol).await;
    }

    /// One unwind attempt on the venue's truth: what is still open in the leg's direction is
    /// closed reduce-only, then read back. Accepted is not closed.
    async fn try_unwind(&mut self, b: &Venue, symbol: &str) {
        let (Some(u), Some(now)) = (self.unwinds.get(symbol).cloned(), self.now) else { return };
        let step = self.syms.get(symbol).and_then(|s| s.step);
        let dust = |q: Decimal| q <= Decimal::ZERO || step.is_some_and(|s| q < s);
        let open_in_leg = |signed: Decimal| grown(u.side, Decimal::ZERO, signed).max(Decimal::ZERO);
        let Some(signed) = self.signed_now(b, symbol).await else {
            self.unwind_failed(symbol, now, "positions unreadable");
            return;
        };
        let qty = u.qty.min(open_in_leg(signed));
        if dust(qty) {
            self.unwind_done(symbol);
            return;
        }
        let mult = Decimal::from(FLATTEN_SLIPPAGE_MULT * (u.attempts.min(4) + 1));
        let close =
            Action { symbol: symbol.to_string(), side: opposite(u.side), qty, reduce_only: true };
        let sent = self.place_one(b, &close, mult, &HashMap::new()).await;
        match self.signed_now(b, symbol).await.map(open_in_leg) {
            Some(left) if dust(left) => self.unwind_done(symbol),
            left => {
                if let (Some(left), Some(u)) = (left, self.unwinds.get_mut(symbol)) {
                    u.qty = u.qty.min(left);
                }
                self.unwind_failed(symbol, now, &format!("order {sent:?}, {left:?} still open"));
            }
        }
    }

    fn unwind_failed(&mut self, symbol: &str, now: Mono, why: &str) {
        let Some(u) = self.unwinds.get_mut(symbol) else { return };
        u.attempts = u.attempts.saturating_add(1);
        let wait = UNWIND_BACKOFF_BASE_SECS
            .saturating_mul(1_u64 << u.attempts.saturating_sub(1).min(16))
            .min(UNWIND_BACKOFF_MAX_SECS);
        u.next_at = now.checked_add(Duration::from_secs(wait)).unwrap_or(now);
        let (attempts, qty) = (u.attempts, u.qty);
        let stuck =
            if attempts >= UNWIND_ESCALATE_AFTER { " STUCK with a one-sided book:" } else { "" };
        let text = format!(
            "simple-funding:{stuck} unwind of {qty} {symbol} failed (attempt {attempts}: {why}); \
             retrying in {wait}s, no new pairs until it closes"
        );
        tracing::error!("{text}");
        self.io.raise(&Self::unwind_key(symbol), text);
    }

    fn unwind_done(&mut self, symbol: &str) {
        if self.unwinds.remove(symbol).is_some() {
            tracing::info!(%symbol, "simple-funding: unwind complete");
            self.io.resolve(&Self::unwind_key(symbol));
        }
    }

    /// Retry every unwind whose backoff is over. `true` when it placed or read anything: the
    /// positions this tick read are then out of date.
    pub(in crate::actor) async fn service_unwinds(&mut self, b: &Venue) -> bool {
        let Some(now) = self.now else { return false };
        let due: Vec<String> = self
            .unwinds
            .iter()
            .filter(|(_, u)| now >= u.next_at)
            .map(|(symbol, _)| symbol.clone())
            .collect();
        let acted = !due.is_empty();
        for symbol in due {
            self.try_unwind(b, &symbol).await;
        }
        acted
    }

    /// Settle the legs whose submit was ambiguous, from the venue's positions this tick read:
    /// confirmed legs stay (matched in USD), anything one-sided beyond [`PAIR_TOLERANCE`] is
    /// queued for unwind. Only a group older than [`AMBIGUOUS_SETTLE_SECS`] is judged.
    pub(in crate::actor) fn reconcile_pending(&mut self, positions: &[Position]) {
        let Some(now) = self.now else { return };
        if self.pending.is_empty() {
            return;
        }
        let settle = Duration::from_secs(AMBIGUOUS_SETTLE_SECS);
        let (due, wait): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| now.checked_duration_since(p.since).is_some_and(|age| age >= settle));
        self.pending = wait;
        for group in due {
            let grew: Vec<(&PendingLeg, Decimal)> = group
                .legs
                .iter()
                .map(|l| {
                    let now_signed = signed_position(positions, &l.symbol);
                    (l, grown(l.side, l.before, now_signed).max(Decimal::ZERO))
                })
                .collect();
            let matched = if grew.len() > 1 {
                grew.iter().map(|(l, q)| usd(*q, l.price)).min().unwrap_or(Decimal::ZERO)
            } else {
                Decimal::ZERO
            };
            for (leg, qty) in grew {
                let value = usd(qty, leg.price);
                if qty <= Decimal::ZERO || value - matched <= value * PAIR_TOLERANCE {
                    continue;
                }
                let raw = (value - matched).checked_div(leg.price).unwrap_or(qty).min(qty);
                let step = self.syms.get(&leg.symbol).and_then(|s| s.step);
                let excess = step.map_or(raw, |s| floor_to_step(raw, s));
                tracing::error!(
                    symbol = %leg.symbol, %excess,
                    "simple-funding: an ambiguous order did commit without its sibling, unwinding"
                );
                self.queue_unwind(&leg.symbol, leg.side, excess);
            }
        }
        if self.pending.is_empty() {
            self.io.resolve("simple_funding_ambiguous_order");
        }
    }

    /// Tracks how long the long and short books have sat past `max_net_pct`; after
    /// [`IMBALANCE_TICKS`] ticks returns the reduce-only exit of the weakest leg (lowest
    /// absolute signal, unknown first) on the heavier side.
    pub(in crate::actor) fn imbalance_exit(&mut self, positions: &[Position]) -> Option<Action> {
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
            if self.recovering {
                tracing::info!("simple-funding: the book is balanced, entries allowed again");
                self.recovering = false;
            }
            return None;
        }
        self.imbalance_ticks = self.imbalance_ticks.saturating_add(1);
        // A restart over a lopsided book (an orphan left by a hold or a crash) sheds at once.
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
        tracing::error!(
            symbol = %weakest.symbol, %skew, limit = %self.config.max_net_pct,
            "simple-funding: books lopsided for too long, exiting the weakest heavy leg"
        );
        Some(Action {
            symbol: weakest.symbol.clone(),
            side: opposite(heavy),
            qty: weakest.size,
            reduce_only: true,
        })
    }
}
