//! The strategy as pure functions: what a closed candle's RSI asks for, whether a price hits the
//! stop, how big an entry may be. No I/O; the actor reads the venue and places what comes back.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal prices, sizes and percentages far below 7.9e28; divisors are checked positive"
)]

use bb_strategy_sdk::Side;
use rust_decimal::Decimal;

/// The signal thresholds (RSI points, 0..=100).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    /// Go long when the RSI crosses above this.
    pub long_above: Decimal,
    /// Go short (if allowed) when the RSI crosses below this.
    pub short_below: Decimal,
    /// A long exits below this, a short above it.
    pub exit_band: Decimal,
    pub allow_short: bool,
}

/// What a closed candle asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Hold,
    Enter(Side),
    Exit,
}

/// The intent at a candle close, from the RSI before and after it.
///
/// - Held: exit once the RSI is past the exit band (below it for a long, above it for a short). An
///   exit is judged on the level, so a position adopted or held across a warm-up exits at the first
///   value past the band; no RSI (warming up) holds, the stop still guards it.
/// - Flat: enter only on a **cross** (`prev` on one side of the threshold, `rsi` past it); a level
///   already past the threshold, or no previous value, is not a cross. No entry while
///   `cooldown_left > 0`; no short unless `allow_short`.
#[must_use]
pub fn on_close(
    p: &Params,
    prev: Option<Decimal>,
    rsi: Option<Decimal>,
    held: Option<Side>,
    cooldown_left: u32,
) -> Intent {
    let Some(rsi) = rsi else { return Intent::Hold };
    match held {
        Some(Side::Buy) if rsi < p.exit_band => return Intent::Exit,
        Some(Side::Sell) if rsi > p.exit_band => return Intent::Exit,
        Some(_) => return Intent::Hold,
        None => {}
    }
    let Some(prev) = prev else { return Intent::Hold };
    if cooldown_left > 0 {
        return Intent::Hold;
    }
    if prev <= p.long_above && rsi > p.long_above {
        return Intent::Enter(Side::Buy);
    }
    if p.allow_short && prev >= p.short_below && rsi < p.short_below {
        return Intent::Enter(Side::Sell);
    }
    Intent::Hold
}

/// The price [`stop_hit`] fires at for a position held on `held` from `entry`: the stop the venue
/// holds for it. `None` without an entry.
#[must_use]
pub fn stop_price(held: Side, entry: Decimal, stop_pct: Decimal) -> Option<Decimal> {
    if entry <= Decimal::ZERO {
        return None;
    }
    let factor = match held {
        Side::Buy => Decimal::ONE.checked_sub(stop_pct)?,
        Side::Sell => Decimal::ONE.checked_add(stop_pct)?,
    };
    entry.checked_mul(factor).filter(|p| *p > Decimal::ZERO)
}

/// `price` is at or past the stop `stop_pct` (a fraction) from `entry` against `held`.
#[must_use]
pub fn stop_hit(held: Side, entry: Decimal, price: Decimal, stop_pct: Decimal) -> bool {
    price > Decimal::ZERO
        && stop_price(held, entry, stop_pct).is_some_and(|stop| match held {
            Side::Buy => price <= stop,
            Side::Sell => price >= stop,
        })
}

pub use bb_strategy_support::floor_to_step;

/// The entry size in base units: `notional_usd` at `price`, never past `max_position` less what
/// is already held, rounded down to `step`. 0 means no entry.
#[must_use]
pub fn entry_qty(
    notional_usd: Decimal,
    price: Decimal,
    step: Decimal,
    max_position: Decimal,
    held_abs: Decimal,
) -> Decimal {
    if price <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let room = (max_position - held_abs).max(Decimal::ZERO);
    floor_to_step((notional_usd / price).min(room), step)
}

#[cfg(test)]
#[path = "decide_tests.rs"]
mod tests;
