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

/// `price` is at or past the stop `stop_pct` (a fraction) from `entry` against `held`.
#[must_use]
pub fn stop_hit(held: Side, entry: Decimal, price: Decimal, stop_pct: Decimal) -> bool {
    if entry <= Decimal::ZERO || price <= Decimal::ZERO {
        return false;
    }
    match held {
        Side::Buy => price <= entry * (Decimal::ONE - stop_pct),
        Side::Sell => price >= entry * (Decimal::ONE + stop_pct),
    }
}

/// `qty` rounded down to a multiple of `step`; 0 for a non-positive step.
#[must_use]
pub fn floor_to_step(qty: Decimal, step: Decimal) -> Decimal {
    if step <= Decimal::ZERO || qty <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    (qty / step).floor() * step
}

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
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn params(allow_short: bool) -> Params {
        Params { long_above: d("60"), short_below: d("40"), exit_band: d("50"), allow_short }
    }

    fn close(prev: Option<&str>, rsi: &str, held: Option<Side>) -> Intent {
        on_close(&params(true), prev.map(d), Some(d(rsi)), held, 0)
    }

    #[test]
    fn a_cross_above_long_above_goes_long() {
        assert_eq!(close(Some("55"), "61", None), Intent::Enter(Side::Buy));
        assert_eq!(close(Some("60"), "60.01", None), Intent::Enter(Side::Buy), "from the line");
    }

    #[test]
    fn a_level_already_past_the_threshold_is_not_a_cross() {
        assert_eq!(close(Some("65"), "70", None), Intent::Hold);
        assert_eq!(close(Some("35"), "30", None), Intent::Hold);
        assert_eq!(
            close(None, "70", None),
            Intent::Hold,
            "the first value has nothing to cross from"
        );
        assert_eq!(close(Some("55"), "60", None), Intent::Hold, "touching is not crossing");
    }

    #[test]
    fn a_cross_below_short_below_goes_short_only_when_allowed() {
        assert_eq!(close(Some("45"), "39", None), Intent::Enter(Side::Sell));
        let p = params(false);
        assert_eq!(on_close(&p, Some(d("45")), Some(d("39")), None, 0), Intent::Hold);
    }

    #[test]
    fn no_rsi_is_no_trade_and_no_exit() {
        let p = params(true);
        assert_eq!(on_close(&p, Some(d("55")), None, None, 0), Intent::Hold);
        assert_eq!(on_close(&p, Some(d("10")), None, Some(Side::Buy), 0), Intent::Hold);
    }

    #[test]
    fn exits_on_the_band() {
        assert_eq!(close(Some("55"), "49.9", Some(Side::Buy)), Intent::Exit);
        assert_eq!(close(Some("55"), "50", Some(Side::Buy)), Intent::Hold);
        assert_eq!(close(Some("45"), "50.1", Some(Side::Sell)), Intent::Exit);
        assert_eq!(close(Some("45"), "50", Some(Side::Sell)), Intent::Hold);
        // Adopted or held across a warm-up: the level alone exits.
        assert_eq!(close(None, "20", Some(Side::Buy)), Intent::Exit);
    }

    #[test]
    fn a_held_position_never_adds_or_reverses() {
        assert_eq!(close(Some("55"), "70", Some(Side::Buy)), Intent::Hold);
        assert_eq!(close(Some("55"), "30", Some(Side::Sell)), Intent::Hold);
    }

    #[test]
    fn cooldown_blocks_entries_not_exits() {
        let p = params(true);
        assert_eq!(on_close(&p, Some(d("55")), Some(d("61")), None, 1), Intent::Hold);
        assert_eq!(on_close(&p, Some(d("55")), Some(d("40")), Some(Side::Buy), 3), Intent::Exit);
    }

    #[test]
    fn the_stop_is_a_fraction_from_entry_against_the_position() {
        let pct = d("0.02");
        assert!(!stop_hit(Side::Buy, d("100"), d("98.01"), pct));
        assert!(stop_hit(Side::Buy, d("100"), d("98"), pct));
        assert!(!stop_hit(Side::Sell, d("100"), d("101.99"), pct));
        assert!(stop_hit(Side::Sell, d("100"), d("102"), pct));
        assert!(!stop_hit(Side::Buy, Decimal::ZERO, d("1"), pct), "unknown entry");
    }

    #[test]
    fn entry_size_is_the_notional_capped_by_max_position_and_floored_to_the_step() {
        // 100 USD at 30 = 3.333..; step 0.1 -> 3.3.
        assert_eq!(entry_qty(d("100"), d("30"), d("0.1"), d("10"), Decimal::ZERO), d("3.3"));
        // max_position 2 caps it.
        assert_eq!(entry_qty(d("100"), d("30"), d("0.1"), d("2"), Decimal::ZERO), d("2"));
        // Already holding 1.5 of 2: 0.5 room.
        assert_eq!(entry_qty(d("100"), d("30"), d("0.1"), d("2"), d("1.5")), d("0.5"));
        // Past the cap: nothing.
        assert_eq!(entry_qty(d("100"), d("30"), d("0.1"), d("2"), d("3")), Decimal::ZERO);
        // Below one step: nothing.
        assert_eq!(entry_qty(d("1"), d("30"), d("0.1"), d("2"), Decimal::ZERO), Decimal::ZERO);
        assert_eq!(
            entry_qty(d("100"), Decimal::ZERO, d("0.1"), d("2"), Decimal::ZERO),
            Decimal::ZERO
        );
    }
}
