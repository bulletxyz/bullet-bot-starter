//! Grid state: fixed level prices indexed `0..levels`, each through a lifecycle (`Dormant` →
//! `Pending` → `Active` → `Dormant`, and `Paused` while its opening order is held back).
//!
//! Invariants:
//!   - Level prices are set once in [`GridState::compute_levels`] and never move.
//!   - At any time, at most one of {buy, sell} rests at a given level.
//!   - `Dormant` = the level has no resting order. It becomes `Pending` when the *adjacent* level
//!     fills ("a buy at N fills → re-arm level N+1 as a sell" and symmetric) or, for a freshly
//!     computed grid, at start (every level but the one nearest the anchor).
//!   - `Pending` with a client id = its Place is in flight (sent, no answer yet): it is not placed
//!     again, and it counts toward the caps as a resting order does. The answer (or a `Failed`)
//!     makes it `Active` or clears the client id; a pause or the stop's clear lets it go, and the
//!     order its answer then reports is cancelled.
//!   - `Paused` = an opening order held back (trend filter, out of range on the profitable side):
//!     it keeps its side and goes back to `Pending` when the pause ends.
//!   - Profit per round trip = `spacing × order_size`; the host's inventory records it on each fill
//!     cycle.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal USD notionals, prices, sizes and bps sit far below 7.9e28; level indices are tiny"
)]

use bb_strategy_sdk::Side;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::Serialize;

use crate::config::TrendFilterConfig;
use crate::levels::ema::Ema;
use crate::plan::Plan;

pub mod ema;

/// Outcome of applying a fill to a grid level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillOutcome {
    /// No level matched the fill — e.g., left over from a prior session.
    Unmatched,
    /// Partial fill; the level's order is still partially live.
    Partial,
    /// Order fully filled; optionally includes the adjacent level we
    /// re-armed (None when we hit the edge of the grid or the neighbour
    /// wasn't dormant).
    Complete { rearm: Option<(usize, Side)> },
}

/// Per-level lifecycle. Flat enum keeps match sites readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub enum LevelState {
    /// No resting order here. Re-armed on an adjacent fill.
    Dormant,
    /// Needs placement on the next `place_pending_orders` pass.
    Pending,
    /// Order is live on the exchange.
    Active,
    /// An opening order held back by a pause; back to `Pending` when it ends.
    Paused,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct GridLevel {
    pub index: usize,
    pub price: Decimal,
    /// `None` only in `Dormant`. Otherwise the side of the resting (or to-be-placed, or paused)
    /// order. Fill events flip this.
    #[schemars(with = "Option<SideName>")]
    pub side: Option<Side>,
    pub state: LevelState,
    pub client_id: Option<String>,
    pub order_id: Option<String>,
    /// Cumulative quantity filled against the order currently at this level.
    /// Reset on re-arm. Compared against the order size to decide when
    /// a partial-fill sequence is complete and the level should transition
    /// to `Dormant`. Serialized for the status API.
    pub filled_qty: Decimal,
}

#[derive(Debug, Default)]
pub struct GridState {
    pub levels: Vec<GridLevel>,
    fast_ema: Option<Ema>,
    slow_ema: Option<Ema>,
    /// The trend filter holds the opening orders back.
    pub paused: bool,
}

impl GridState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable trend-filter EMAs at construction time.
    #[must_use]
    #[expect(clippy::cast_precision_loss, reason = "EMA window seconds are far below 2^52")]
    pub fn with_trend_filter(mut self, cfg: &TrendFilterConfig) -> Self {
        self.fast_ema = Some(Ema::new(cfg.fast_secs as f64));
        self.slow_ema = Some(Ema::new(cfg.slow_secs as f64));
        self
    }

    /// Fold past closes into the trend filter's EMAs, oldest first, each at its close time
    /// (unix ms), so a market already trending at the start reads as one. The pause is decided on
    /// the next live mid, as always. A no-op without the filter.
    pub fn warm_trend(&mut self, closes: &[(u64, Decimal)]) {
        let (Some(fast), Some(slow)) = (self.fast_ema.as_mut(), self.slow_ema.as_mut()) else {
            return;
        };
        for (at_ms, close) in closes {
            if let Some(sample) = close.to_f64() {
                fast.update(sample, *at_ms);
                slow.update(sample, *at_ms);
            }
        }
    }

    /// Populate `self.levels` from the plan: buys at or below the anchor, sells above, the level
    /// within half a spacing of the anchor dormant (no order until a neighbour's fill re-arms it).
    pub fn compute_levels(&mut self, plan: &Plan, tick: Option<Decimal>) {
        self.levels = plan
            .sides(tick)
            .into_iter()
            .enumerate()
            .map(|(i, (price, side))| GridLevel {
                index: i,
                price,
                side,
                state: if side.is_some() { LevelState::Pending } else { LevelState::Dormant },
                client_id: None,
                order_id: None,
                filled_qty: Decimal::ZERO,
            })
            .collect();
    }

    /// Find the level that matches an incoming fill. Falls back from
    /// `client_id` to `order_id` since Bullet doesn't return oids synchronously.
    pub fn find_fill_target(&mut self, client_id: Option<&str>, order_id: &str) -> Option<usize> {
        self.levels.iter().position(|l| {
            if l.state != LevelState::Active {
                return false;
            }
            match client_id {
                Some(cid) => l.client_id.as_deref() == Some(cid),
                None => l.order_id.as_deref() == Some(order_id),
            }
        })
    }

    /// Accumulate a fill against level `idx` and return whether this fill
    /// completed the order at that level.
    ///
    /// The level's order is considered complete when cumulative
    /// `filled_qty >= order_size`. Until then the level stays `Active` and
    /// no re-arm fires — a single order can take multiple partial fills
    /// on venues that don't match it atomically. Once complete, the level
    /// transitions to `Dormant` and the neighbour to re-arm (if any) is returned.
    pub fn record_fill(&mut self, idx: usize, qty: Decimal, order_size: Decimal) -> FillOutcome {
        let Some(lvl) = self.levels.get_mut(idx) else {
            return FillOutcome::Unmatched;
        };
        lvl.filled_qty += qty;
        if lvl.filled_qty < order_size {
            return FillOutcome::Partial;
        }
        // Order complete: drop to Dormant so `find_fill_target` won't match
        // any late-arriving duplicate fills and so the tick reconcile sees
        // "no live order here."
        let filled_side = lvl.side;
        lvl.state = LevelState::Dormant;
        lvl.side = None;
        lvl.client_id = None;
        lvl.order_id = None;
        lvl.filled_qty = Decimal::ZERO;

        match filled_side {
            Some(side) => FillOutcome::Complete { rearm: self.rearm_adjacent(idx, side) },
            None => FillOutcome::Complete { rearm: None },
        }
    }

    /// Re-arm the adjacent level (N+1 for a filled buy, N-1 for a filled
    /// sell) with the opposite side, but only if it's currently Dormant.
    /// An already-resting adjacent is left alone to avoid self-crossing or
    /// duplicate placement.
    fn rearm_adjacent(&mut self, idx: usize, filled_side: Side) -> Option<(usize, Side)> {
        let target_idx = match filled_side {
            Side::Buy => idx.checked_add(1)?,
            Side::Sell => idx.checked_sub(1)?,
        };
        let target = self.levels.get_mut(target_idx)?;
        if target.state != LevelState::Dormant {
            return None;
        }
        let replacement_side = filled_side.opposite();
        target.state = LevelState::Pending;
        target.side = Some(replacement_side);
        Some((target_idx, replacement_side))
    }

    /// Update trend-filter EMAs from a fresh mid observation and return the
    /// current `(divergence_bps, paused)`. Noop / returns `(0, paused)` when
    /// the filter isn't configured.
    #[expect(clippy::cast_precision_loss, reason = "EMA windows and elapsed ms are far below 2^52")]
    pub fn update_trend_filter(
        &mut self,
        mid: Decimal,
        now_ms: u64,
        cfg: &TrendFilterConfig,
    ) -> (f64, bool) {
        let Some(sample) = mid.to_f64() else {
            return (0.0, self.paused);
        };
        let fast = self.fast_ema.get_or_insert_with(|| Ema::new(cfg.fast_secs as f64));
        let f = fast.update(sample, now_ms);
        let slow = self.slow_ema.get_or_insert_with(|| Ema::new(cfg.slow_secs as f64));
        let s = slow.update(sample, now_ms);
        if s == 0.0 {
            return (0.0, self.paused);
        }
        let divergence_bps = (f - s) / s * 10_000.0;
        let threshold = cfg.pause_divergence_bps.to_f64().unwrap_or(f64::INFINITY);
        self.paused = divergence_bps.abs() > threshold;
        (divergence_bps, self.paused)
    }

    pub fn active_count(&self) -> usize {
        self.levels.iter().filter(|l| l.state == LevelState::Active).count()
    }

    /// What the orders on `side` still hold: the resting ones and the ones in flight (`Pending`
    /// with a client id), which may rest as soon as their answer comes.
    pub fn active_remaining(&self, side: Side, order_size: Decimal) -> Decimal {
        let holds = |l: &GridLevel| {
            l.state == LevelState::Active
                || (l.state == LevelState::Pending && l.client_id.is_some())
        };
        self.levels
            .iter()
            .filter(|l| holds(l) && l.side == Some(side))
            .map(|l| (order_size - l.filled_qty).max(Decimal::ZERO))
            .sum()
    }

    /// The levels whose order closes part of `position` (the side against it, nearest the price
    /// first, as far as the position goes); every other armed level opens.
    pub fn closing(&self, position: Decimal, order_size: Decimal) -> Vec<bool> {
        let mut closing = vec![false; self.levels.len()];
        let side = match position.cmp(&Decimal::ZERO) {
            std::cmp::Ordering::Greater => Side::Sell,
            std::cmp::Ordering::Less => Side::Buy,
            std::cmp::Ordering::Equal => return closing,
        };
        let mut idx: Vec<usize> = (0..self.levels.len())
            .filter(|&i| {
                let l = &self.levels[i];
                l.side == Some(side) && l.state != LevelState::Dormant
            })
            .collect();
        // Nearest the price first: the lowest sells, the highest buys.
        if side == Side::Buy {
            idx.reverse();
        }
        let mut left = position.abs();
        for i in idx {
            let qty = (order_size - self.levels[i].filled_qty).max(Decimal::ZERO);
            if qty > left {
                break;
            }
            left -= qty;
            closing[i] = true;
        }
        closing
    }

    /// Hold level `i`'s order back: it keeps its side, loses its order.
    pub fn pause(&mut self, i: usize) {
        if let Some(l) = self.levels.get_mut(i)
            && matches!(l.state, LevelState::Active | LevelState::Pending)
        {
            l.state = LevelState::Paused;
            l.client_id = None;
            l.order_id = None;
            l.filled_qty = Decimal::ZERO;
        }
    }

    /// Every paused level back to `Pending`.
    pub fn unpause_all(&mut self) {
        for l in &mut self.levels {
            if l.state == LevelState::Paused {
                l.state = LevelState::Pending;
            }
        }
    }

    /// Every level dormant, no order anywhere: the grid has stopped.
    pub fn clear_all(&mut self) {
        for l in &mut self.levels {
            l.state = LevelState::Dormant;
            l.side = None;
            l.client_id = None;
            l.order_id = None;
            l.filled_qty = Decimal::ZERO;
        }
    }
}

#[cfg(test)]
#[path = "levels_tests.rs"]
mod tests;

/// How a [`Side`] serializes, for the report's schema (`Side` is bb-core's).
#[derive(schemars::JsonSchema)]
#[schemars(rename_all = "lowercase")]
#[expect(dead_code, reason = "only its schema is used")]
enum SideName {
    Buy,
    Sell,
}
