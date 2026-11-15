//! The funding arb's `decide`, ported from the built-in: a [`MarketView`] in, one [`Target`] per
//! symbol out (direction and notional per leg). Pure: no I/O, no clock.
//!
//! Rules: short the venue whose trailing funding is higher; enter or grow while the carry over
//! `expected_hold_hours` beats the measured round trip; exit only when the signal flips sign; size
//! to `min(notional_per_symbol, max_oi_pct x OI)`, best net carry first within the leverage budget.
//!
//! The types are the built-in's view of the market. What builds a view from the venues (the signal,
//! the cost) is [`crate::view`], new code of this plugin; the built-in's is the bot's.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal USD notionals, prices, sizes and bps sit far below 7.9e28; constant divisors; small counters"
)]

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Which venue is short: the other one is long.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    ShortBullet,
    LongBullet,
}

/// What a venue holds of one symbol: signed base units (short negative).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Leg {
    pub base: Decimal,
}

/// The settings `decide` uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Params {
    pub notional_per_symbol: Decimal,
    pub max_positions: usize,
    pub max_oi_pct: Decimal,
    pub max_leverage: Decimal,
    pub expected_hold_hours: u64,
}

/// One symbol as `decide` sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolView {
    pub symbol: String,
    /// Bullet funding minus Hyperliquid's, per 8h; `None` = unknown.
    pub signal_8h: Option<Decimal>,
    /// Round trip as a fraction of notional; `None` = unknown.
    pub round_trip_cost: Option<Decimal>,
    /// Mark price; 0 = unknown.
    pub mark: Decimal,
    /// Bullet open interest in USD; `None` = unknown.
    pub bullet_oi_usd: Option<Decimal>,
    pub leg_bullet: Leg,
    pub leg_hl: Leg,
}

/// Every symbol and the accounts, as `decide` sees them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketView {
    pub symbols: Vec<SymbolView>,
    pub equity_bullet: Option<Decimal>,
    pub equity_hl: Option<Decimal>,
    /// Gross notional held outside the symbols above, per venue.
    pub foreign_notional_bullet: Decimal,
    pub foreign_notional_hl: Decimal,
    pub params: Params,
}

/// What a symbol should hold: a direction and the USD notional per leg (0 = flat).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub symbol: String,
    pub direction: Direction,
    pub notional_usd: Decimal,
}

/// One target per symbol in `view`, in view order. A symbol left out would be read as target 0,
/// so none is.
#[must_use]
pub fn decide(view: &MarketView) -> Vec<Target> {
    let p = &view.params;
    let plans: Vec<Plan> = view.symbols.iter().map(|s| plan(s, p)).collect();

    // Leverage budget: max_leverage x equity - foreign notional, tightest venue; unknown equity =
    // no room.
    let budget = match (view.equity_bullet, view.equity_hl) {
        (Some(eq_b), Some(eq_h)) => (p.max_leverage * eq_b - view.foreign_notional_bullet)
            .min(p.max_leverage * eq_h - view.foreign_notional_hl),
        _ => Decimal::ZERO,
    };
    // Held positions are kept first (up to their own target), whatever the budget.
    let mut granted: Vec<Decimal> = plans.iter().map(|pl| pl.keep).collect();
    let mut used: Decimal = granted.iter().sum();
    let mut open = granted.iter().filter(|g| **g > Decimal::ZERO).count();

    // Grows and entries share what is left, best net carry first.
    let mut order: Vec<usize> =
        (0..plans.len()).filter(|&i| plans[i].want > plans[i].keep).collect();
    order.sort_by(|&a, &b| plans[b].carry.cmp(&plans[a].carry));
    for i in order {
        let room = budget - used;
        if room <= Decimal::ZERO {
            break;
        }
        let new_position = granted[i].is_zero();
        // max_positions bounds the count of non-zero targets.
        if new_position && open >= p.max_positions {
            continue;
        }
        let add = (plans[i].want - plans[i].keep).min(room);
        granted[i] += add;
        used += add;
        if new_position {
            open += 1;
        }
    }

    plans
        .into_iter()
        .zip(granted)
        .map(|(pl, notional_usd)| Target {
            symbol: pl.symbol,
            direction: pl.direction,
            notional_usd,
        })
        .collect()
}

/// One symbol's wish before the shared budget: `keep` is granted outright, the rest of `want`
/// competes on `carry`.
struct Plan {
    symbol: String,
    direction: Direction,
    keep: Decimal,
    want: Decimal,
    carry: Option<Decimal>,
}

fn plan(s: &SymbolView, p: &Params) -> Plan {
    // Bullet short = ShortBullet; base 0 = flat.
    let held_dir = match s.leg_bullet.base.cmp(&Decimal::ZERO) {
        std::cmp::Ordering::Less => Some(Direction::ShortBullet),
        std::cmp::Ordering::Greater => Some(Direction::LongBullet),
        std::cmp::Ordering::Equal => None,
    };
    // Direction: short the venue that pays more. Signal = Bullet - HL, so > 0 means short Bullet.
    let signal_dir = s.signal_8h.and_then(|sig| match sig.cmp(&Decimal::ZERO) {
        std::cmp::Ordering::Greater => Some(Direction::ShortBullet),
        std::cmp::Ordering::Less => Some(Direction::LongBullet),
        std::cmp::Ordering::Equal => None,
    });
    // Net carry over the hold after the round trip; unknown if the signal or the cost is.
    let hold_hours = Decimal::from(p.expected_hold_hours);
    let carry = s
        .signal_8h
        .zip(s.round_trip_cost)
        .map(|(sig, cost)| sig.abs() / Decimal::from(8) * hold_hours - cost);
    // Hurdle: the carry beats the round trip.
    let edge = carry.is_some_and(|c| c > Decimal::ZERO);
    // Cap: notional_per_symbol and max_oi_pct of Bullet OI; unknown OI = no known cap.
    let cap = s.bullet_oi_usd.map(|oi| p.notional_per_symbol.min(p.max_oi_pct * oi));
    // Current notional per leg; unknown when the mark is 0.
    let mark_known = s.mark > Decimal::ZERO;
    let current = mark_known.then(|| s.leg_bullet.base.abs() * s.mark);

    let (direction, keep, want) = match held_dir {
        // Flat: enter at the cap only with a known signal, cost, mark and OI and the hurdle met.
        None => {
            let want = match cap {
                Some(cap) if edge && mark_known => cap,
                _ => Decimal::ZERO,
            };
            (signal_dir.unwrap_or(Direction::ShortBullet), Decimal::ZERO, want)
        }
        // Signal flipped against the held side: exit.
        Some(dir) if signal_dir.is_some_and(|sd| sd != dir) => (dir, Decimal::ZERO, Decimal::ZERO),
        Some(dir) => {
            // Hold: trim to a known cap, never grow. With no mark, the cap bounds the hold.
            let hold_at = match (current, cap) {
                (Some(cur), Some(cap)) => cur.min(cap),
                (Some(cur), None) => cur,
                (None, Some(cap)) => cap,
                (None, None) => p.notional_per_symbol,
            };
            // Edge still there and mark and OI known: aim for the cap (grow from the budget).
            let want = match cap {
                Some(cap) if edge && mark_known => cap,
                _ => hold_at,
            };
            (dir, hold_at, want)
        }
    };
    Plan { symbol: s.symbol.clone(), direction, keep, want, carry }
}
