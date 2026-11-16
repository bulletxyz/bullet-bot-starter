//! From targets to orders, behind the caps. `decide` picks targets; this clamps them again
//! (leg size, open interest, leverage, positions, entries per tick, stale data, unknown
//! inputs) so a bug in `decide` can size a position wrongly but cannot exceed a limit.
//! Pure: the actor reads the venue and places what comes back.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal USD notionals, prices and sizes sit far below 7.9e28; the step divisor is checked positive"
)]

use std::cmp::Ordering;

use bb_strategy_sdk::Side;
use rust_decimal::Decimal;

use crate::limits::{ENTRIES_PER_SIDE_PER_TICK, MIN_GROW_SHARE};

/// The caps a grow is clamped to.
#[derive(Debug, Clone, PartialEq)]
pub struct Caps {
    pub notional_per_symbol: Decimal,
    pub max_positions: usize,
    pub max_leverage: Decimal,
    pub max_oi_pct: Decimal,
}

/// One symbol: what the venue holds and what `decide` wants.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub symbol: String,
    /// Valuation price (the mark, else the entry price). 0 = unknown.
    pub mark: Decimal,
    /// The venue position, signed base units (long positive).
    pub held_base: Decimal,
    /// Size step. `None` = unknown: no grow.
    pub step: Option<Decimal>,
    /// Open interest in base units. `None` = unknown: no grow.
    pub oi_base: Option<Decimal>,
    pub fresh: bool,
    /// Desired signed notional, USD.
    pub target_usd: Decimal,
}

/// One IOC order to place.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub symbol: String,
    pub side: Side,
    pub qty: Decimal,
    pub reduce_only: bool,
}

fn side_of(signed: Decimal) -> Option<Side> {
    match signed.cmp(&Decimal::ZERO) {
        Ordering::Greater => Some(Side::Buy),
        Ordering::Less => Some(Side::Sell),
        Ordering::Equal => None,
    }
}

pub(crate) fn opposite(side: Side) -> Side {
    match side {
        Side::Buy => Side::Sell,
        Side::Sell => Side::Buy,
    }
}

/// `qty` rounded down to a multiple of `step`.
pub(crate) fn floor_to_step(qty: Decimal, step: Decimal) -> Decimal {
    if step <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    (qty / step).floor() * step
}

/// Exits first (reduce-only, the whole position), then grows in row order.
///
/// A target of 0, or against the position, only exits: a reversal takes two ticks.
/// A grow needs fresh data, known equity, step and open interest, and is clipped to
/// `notional_per_symbol`, `max_oi_pct` of open interest and the leverage room; a new
/// symbol also needs a free `max_positions` slot, and at most
/// [`ENTRIES_PER_SIDE_PER_TICK`] new symbols open per side per tick.
pub fn plan(
    rows: &[Row],
    equity: Option<Decimal>,
    foreign_gross_usd: Decimal,
    caps: &Caps,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut exiting = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let Some(held) = side_of(r.held_base) else { continue };
        if side_of(r.target_usd) != Some(held) {
            actions.push(Action {
                symbol: r.symbol.clone(),
                side: opposite(held),
                qty: r.held_base.abs(),
                reduce_only: true,
            });
            exiting.push(i);
        }
    }

    let Some(equity) = equity else { return actions };
    let gross = rows.iter().filter(|r| r.mark > Decimal::ZERO).fold(Decimal::ZERO, |sum, r| {
        sum.saturating_add(r.held_base.abs().checked_mul(r.mark).unwrap_or(Decimal::MAX))
    });
    let mut room = caps.max_leverage * equity - foreign_gross_usd - gross;
    let mut open = rows
        .iter()
        .enumerate()
        .filter(|(i, r)| !r.held_base.is_zero() && !exiting.contains(i))
        .count();
    let (mut new_buys, mut new_sells) = (0, 0);

    for (i, r) in rows.iter().enumerate() {
        let Some(want) = side_of(r.target_usd) else { continue };
        if exiting.contains(&i) || !r.fresh || r.mark <= Decimal::ZERO {
            continue;
        }
        let (Some(step), Some(oi)) = (r.step, r.oi_base) else { continue };
        let Some(oi_cap) = caps.max_oi_pct.checked_mul(oi).and_then(|v| v.checked_mul(r.mark))
        else {
            continue;
        };
        let target = r.target_usd.abs().min(caps.notional_per_symbol).min(oi_cap);
        let held_usd = r.held_base.abs().checked_mul(r.mark).unwrap_or(Decimal::MAX);
        let delta = target - held_usd;
        if delta < target * MIN_GROW_SHARE || delta <= Decimal::ZERO {
            continue;
        }
        let new_position = r.held_base.is_zero();
        if new_position {
            let entries = if want == Side::Buy { &new_buys } else { &new_sells };
            if open >= caps.max_positions || *entries >= ENTRIES_PER_SIDE_PER_TICK {
                continue;
            }
        }
        let delta = delta.min(room);
        let qty = floor_to_step(delta / r.mark, step);
        if qty <= Decimal::ZERO {
            continue;
        }
        if new_position {
            open += 1;
            if want == Side::Buy {
                new_buys += 1;
            } else {
                new_sells += 1;
            }
        }
        room -= qty * r.mark;
        actions.push(Action { symbol: r.symbol.clone(), side: want, qty, reduce_only: false });
    }
    actions
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
