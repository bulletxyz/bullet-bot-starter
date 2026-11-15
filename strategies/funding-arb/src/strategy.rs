//! The strategy: **this is the file to change.**
//!
//! `decide` is a pure function: it reads a [`MarketView`] and returns one [`Target`]
//! per symbol (direction and notional per leg). No I/O, no broker, no clock. The
//! safety executor in bb-funding (`crates/engine/funding/src/executor/`) clamps whatever comes
//! back to the hard caps, turns it into clipped orders, and owns hedging, reconcile and
//! flattening, so a bug here can size a position wrongly but cannot bypass a limit. See
//! this crate's AGENTS.md section "Funding arb: strategy vs safety executor".
//!
//! Rules (spec §4): short the venue whose trailing funding is higher; enter or grow
//! while the carry over `expected_hold_hours` beats the measured round trip; exit
//! only when the signal flips sign; size to `min(notional_per_symbol,
//! max_oi_pct × OI)`, best net carry first within the leverage budget.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal USD notionals, prices, sizes and bps sit far below 7.9e28; constant divisors; small counters"
)]

use bb_strategy_sdk::targets::{Direction, MarketView, Params, SymbolView, Target};
use bb_strategy_sdk::{
    Event, Intent, StrategyAdapter, StrategyError, StrategyReport, View, typed_report,
};
use rust_decimal::Decimal;

/// The funding arb blueprint as a [`StrategyAdapter`]: it answers every [`Event::MarketView`] with
/// [`Intent::Targets`] of [`decide`] and does nothing else; the host's executor carries them out.
#[derive(Debug, Clone, Copy, Default)]
pub struct FundingArb;

/// The funding arb has nothing of its own to report: the executor reports its own state.
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct FundingArbReport {}

impl StrategyReport for FundingArbReport {
    typed_report!();

    fn headline(&self) -> String {
        String::new()
    }
}

impl StrategyAdapter for FundingArb {
    type Report = FundingArbReport;

    fn start(&mut self, _view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        Ok(Vec::new())
    }

    fn on_event(&mut self, event: &Event, _view: &View<'_>) -> Vec<Intent> {
        match event {
            Event::MarketView(view) => vec![Intent::Targets(decide(view))],
            _ => Vec::new(),
        }
    }

    fn stop(&mut self, _view: &View<'_>) -> Vec<Intent> {
        Vec::new()
    }

    fn report(&self) -> FundingArbReport {
        FundingArbReport {}
    }
}

/// One target per symbol in `view`, in view order. A symbol left out would be read
/// as target 0, so none is.
pub fn decide(view: &MarketView) -> Vec<Target> {
    let p = &view.params;
    let plans: Vec<Plan> = view.symbols.iter().map(|s| plan(s, p)).collect();

    // Leverage budget: max_leverage × equity − foreign notional, tightest venue; unknown equity =
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

/// One symbol's wish before the shared budget: `keep` is granted outright, the rest
/// of `want` competes on `carry`.
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
    // Direction: short the venue that pays more. Signal = Bullet − HL, so > 0 means short Bullet.
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

#[cfg(test)]
#[path = "strategy_tests.rs"]
mod tests;
