//! The strategy: **this is the file to change.**
//!
//! `decide` is a pure function: a [`MarketView`] in, one [`Target`] per symbol out. No
//! I/O, no broker, no clock. The actor (`actor.rs`) clamps whatever comes back to the caps
//! again (`plan.rs`) before it trades.
//!
//! Rules: rank symbols by the trailing mean of settled funding (`signal_8h`, per 8h).
//! A positive signal pays shorts, a negative one pays longs. Enter a flat symbol only when
//! the carry over `expected_hold_hours` (`|signal| / 8 × hours`) beats the measured round
//! trip; keep a held one until the signal flips sign or the carry falls under half the
//! round trip it cost to enter (`entry_cost`, else today's: hysteresis). Short the strongest
//! positive signals and long the strongest negative ones, `top_k` per side, equal USD per leg,
//! never more than the leverage budget and `max_positions` allow, and keep the long and short books
//! within `max_net_pct` of each other. An unknown signal (or cost) blocks an entry and never forces
//! an exit.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal USD notionals, rates and counts sit far below 7.9e28; divisors are constants or checked nonzero"
)]

use std::cmp::Ordering;

use rust_decimal::Decimal;

use crate::limits::{EXIT_HURDLE_SHARE, MIN_ENTRY_SHARE, TOP_UP_BELOW};

/// The config knobs `decide` uses.
#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    pub notional_per_symbol: Decimal,
    pub top_k: usize,
    pub max_positions: usize,
    pub max_net_pct: Decimal,
    pub max_oi_pct: Decimal,
    pub max_leverage: Decimal,
    pub expected_hold_hours: u64,
    pub min_depth_usd: Decimal,
}

/// One market as `decide` sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolView {
    /// Bullet symbol, e.g. `"BTC-USD"`.
    pub symbol: String,
    /// Trailing mean of settled funding, per 8h. `None` = not enough history.
    pub signal_8h: Option<Decimal>,
    /// Round trip as a fraction of notional. `None` = unknown fee or an uncostable book.
    pub round_trip_cost: Option<Decimal>,
    /// Mark price; 0 = unknown.
    pub mark: Decimal,
    /// Open interest in USD. `None` = unknown.
    pub oi_usd: Option<Decimal>,
    /// The thinner book side in USD. `None` = unknown.
    pub depth_usd: Option<Decimal>,
    /// The mark and book are recent.
    pub fresh: bool,
    /// The venue position's signed notional, USD (long positive).
    pub held_usd: Decimal,
    /// What this held position's entry cost as a fraction of notional (fees + book walk when it
    /// was opened). `None` = flat, or unknown (a restart): the exit hurdle then falls back to
    /// today's round trip.
    pub entry_cost: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarketView {
    pub symbols: Vec<SymbolView>,
    /// Account equity, USD. `None` = unknown: no entries.
    pub equity: Option<Decimal>,
    /// Gross notional of positions outside the universe, USD (counts against leverage).
    pub foreign_gross_usd: Decimal,
    pub params: Params,
}

/// Desired signed notional for one symbol: positive long, negative short, 0 flat.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub symbol: String,
    pub notional_usd: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leg {
    Long,
    Short,
}

impl Leg {
    /// A positive signal pays shorts, a negative one pays longs; zero pays nobody.
    fn of_signal(signal: Decimal) -> Option<Self> {
        match signal.cmp(&Decimal::ZERO) {
            Ordering::Greater => Some(Self::Short),
            Ordering::Less => Some(Self::Long),
            Ordering::Equal => None,
        }
    }

    fn of_notional(usd: Decimal) -> Option<Self> {
        match usd.cmp(&Decimal::ZERO) {
            Ordering::Greater => Some(Self::Long),
            Ordering::Less => Some(Self::Short),
            Ordering::Equal => None,
        }
    }

    fn signed(self, usd: Decimal) -> Decimal {
        match self {
            Self::Long => usd,
            Self::Short => -usd,
        }
    }
}

/// Funding earned over the assumed hold, as a fraction of notional.
fn carry(signal_8h: Decimal, hold_hours: u64) -> Decimal {
    signal_8h.abs() / Decimal::from(8) * Decimal::from(hold_hours)
}

/// What stays of a held position: all of it, or nothing (flipped or no longer worth it).
fn keep(s: &SymbolView, p: &Params) -> Decimal {
    let Some(held) = Leg::of_notional(s.held_usd) else { return Decimal::ZERO };
    let Some(signal) = s.signal_8h else { return s.held_usd };
    if Leg::of_signal(signal) != Some(held) {
        return Decimal::ZERO;
    }
    // The exit band hangs off what the position cost to open, not off today's book: a spread
    // that widens for a minute would otherwise exit a pair that a narrow book re-enters, paying
    // the fees both ways each time. Unknown entry cost (a restart) falls back to today's.
    match s.entry_cost.or(s.round_trip_cost) {
        Some(cost) if carry(signal, p.expected_hold_hours) < cost * EXIT_HURDLE_SHARE => {
            Decimal::ZERO
        }
        _ => s.held_usd,
    }
}

/// An entry, or a top-up of a held leg that a partial fill left short.
struct Add {
    index: usize,
    leg: Leg,
    /// The leg's target size, USD.
    size: Decimal,
    /// Extra gross this adds, USD.
    extra: Decimal,
    carry: Decimal,
    new_position: bool,
}

fn candidate(index: usize, s: &SymbolView, kept: Decimal, p: &Params) -> Option<Add> {
    let signal = s.signal_8h?;
    let leg = Leg::of_signal(signal)?;
    if s.held_usd != Decimal::ZERO && kept == Decimal::ZERO {
        return None;
    }
    let cost = s.round_trip_cost?;
    let carry = carry(signal, p.expected_hold_hours);
    let oi = s.oi_usd.filter(|oi| *oi > Decimal::ZERO)?;
    let enough_depth = s.depth_usd.is_some_and(|d| d >= p.min_depth_usd);
    if !s.fresh || s.mark <= Decimal::ZERO || carry <= cost || !enough_depth {
        return None;
    }
    let size = p.notional_per_symbol.min(p.max_oi_pct * oi);
    if size < p.notional_per_symbol * MIN_ENTRY_SHARE {
        return None;
    }
    let held = kept.abs();
    if held > Decimal::ZERO && held >= size * TOP_UP_BELOW {
        return None;
    }
    Some(Add { index, leg, size, extra: size - held, carry, new_position: held.is_zero() })
}

/// `|long - short| / (long + short)`; 0 for an empty book.
fn imbalance(long: Decimal, short: Decimal) -> Decimal {
    let total = long + short;
    if total.is_zero() { Decimal::ZERO } else { (long - short).abs() / total }
}

fn books(targets: &[Decimal]) -> (Decimal, Decimal) {
    let long = targets.iter().filter(|t| **t > Decimal::ZERO).sum();
    let short = targets.iter().filter(|t| **t < Decimal::ZERO).map(|t| -*t).sum();
    (long, short)
}

/// Best first: carry high to low, then symbol for a stable order.
fn ranked(mut adds: Vec<Add>, view: &MarketView) -> std::collections::VecDeque<Add> {
    adds.sort_by(|a, b| {
        b.carry
            .cmp(&a.carry)
            .then_with(|| view.symbols[a.index].symbol.cmp(&view.symbols[b.index].symbol))
    });
    adds.into()
}

/// One target per symbol in `view`, in view order. A symbol left out would be read as
/// target 0, so none is.
pub fn decide(view: &MarketView) -> Vec<Target> {
    let p = &view.params;
    let kept: Vec<Decimal> = view.symbols.iter().map(|s| keep(s, p)).collect();
    let mut targets = kept.clone();

    let adds: Vec<Add> = match view.equity {
        Some(_) => view
            .symbols
            .iter()
            .enumerate()
            .filter_map(|(i, s)| candidate(i, s, kept[i], p))
            .collect(),
        None => Vec::new(),
    };
    let (shorts, longs): (Vec<Add>, Vec<Add>) = adds.into_iter().partition(|a| a.leg == Leg::Short);
    let (mut shorts, mut longs) = (ranked(shorts, view), ranked(longs, view));

    let count = |leg: Leg, from: &[Decimal]| {
        from.iter().filter(|t| Leg::of_notional(**t) == Some(leg)).count()
    };
    let mut open = kept.iter().filter(|t| !t.is_zero()).count();
    let (mut n_long, mut n_short) = (count(Leg::Long, &kept), count(Leg::Short, &kept));
    let kept_gross: Decimal = kept.iter().map(Decimal::abs).sum();
    let mut room = view
        .equity
        .map_or(Decimal::ZERO, |eq| p.max_leverage * eq - view.foreign_gross_usd - kept_gross);

    // Best first, one side then the other, so a basket grows on both sides together.
    let long_first = match (longs.front(), shorts.front()) {
        (Some(l), Some(s)) => l.carry > s.carry,
        (Some(_), None) => true,
        _ => false,
    };
    let mut turn_long = long_first;
    let mut granted: Vec<Add> = Vec::new();
    while !shorts.is_empty() || !longs.is_empty() {
        let from_long = if longs.is_empty() {
            false
        } else if shorts.is_empty() {
            true
        } else {
            turn_long
        };
        turn_long = !from_long;
        let Some(add) = (if from_long { longs.pop_front() } else { shorts.pop_front() }) else {
            continue;
        };
        if add.extra > room {
            continue;
        }
        if add.new_position {
            let side_count = if add.leg == Leg::Long { &mut n_long } else { &mut n_short };
            if open >= p.max_positions || *side_count >= p.top_k {
                continue;
            }
            *side_count += 1;
            open += 1;
        }
        room -= add.extra;
        granted.push(add);
    }

    let held_net = {
        let (l, s) = books(&kept);
        (l - s).abs()
    };
    for add in &granted {
        targets[add.index] = add.leg.signed(add.size);
    }
    // Neutrality: while the book is lopsided past the bound and its net exposure is larger
    // than what is already held, or no smaller, drop the lowest-ranked addition on the heavy side.
    loop {
        let (l, s) = books(&targets);
        if imbalance(l, s) <= p.max_net_pct || (l - s).abs() < held_net {
            break;
        }
        let heavy = if l > s { Leg::Long } else { Leg::Short };
        let Some(pos) = granted.iter().rposition(|a| a.leg == heavy) else { break };
        let dropped = granted.remove(pos);
        targets[dropped.index] = kept[dropped.index];
    }

    view.symbols
        .iter()
        .zip(targets)
        .map(|(s, notional_usd)| Target { symbol: s.symbol.clone(), notional_usd })
        .collect()
}

#[cfg(test)]
#[path = "decide_tests.rs"]
mod tests;
