//! The grid's geometry from market data (Binance's method): the range from the volatility of
//! daily returns, the step from the hourly ATR, the level count from the range, the dollars and
//! an order-count cap. Pure: the candles come in from the build, the mid, tick and lot from the
//! host's view.
//!
//! - **Range**: `mid ± h`, `h = clamp(3σ)` where σ is the standard deviation of the daily close to
//!   close returns over the preset's lookback (Tight 7 d, Normal 30 d, Wide 90 d), clamped to the
//!   preset's band (Tight 2-29 %, Normal 3-47 %, Wide 4-60 % of the mid). A calmer lookback gives a
//!   tighter range; the band keeps a dead-calm week from a range a tick wide and a crash from a
//!   range down to zero.
//! - **Step**: `max(0.77 × ATR(14) on 1 h candles, 5 ticks, 0.1 % of the mid)`, up to the tick.
//! - **Levels**: `min(range / step + 1, dollars / (lot × mid), MAX_LEVELS)`; the sizing then drops
//!   levels until each order is at least one lot.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal prices, ticks and percentages sit far below 7.9e28; level counts are tiny"
)]

use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};

/// The most levels a grid places: Bullet allows 100 open orders a market (`MAX_NUM_ORDERS`), and
/// half of that leaves room for the re-arms and a close in flight. Its start places them all in
/// one batch, far under the order path's default 600 orders a minute.
pub const MAX_LEVELS: usize = 50;
/// The fewest: one empty level at the mid and at least one order on each side of it.
pub const MIN_LEVELS: usize = 4;
/// ATR window on the hourly candles.
pub const ATR_PERIOD: usize = 14;

/// One past candle (bb-core's `ports::Candle`). A plugin gets none (the world has no candle
/// read), so the range and the levels are never derived in the port; the code is kept as the
/// built-in's for the day it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candle {
    /// When it opened (unix ms, a multiple of the interval).
    pub open_ms: u64,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    /// Traded in it, in the base asset (zero when the source does not say).
    pub volume: Decimal,
}

/// How wide the grid is: the lookback of the volatility that sets it, and the band it is held to.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RangePreset {
    Tight,
    #[default]
    Normal,
    Wide,
}

impl RangePreset {
    pub const NAMES: &'static [&'static str] = &["tight", "normal", "wide"];

    #[must_use]
    pub fn lookback_days(self) -> u32 {
        match self {
            Self::Tight => 7,
            Self::Normal => 30,
            Self::Wide => 90,
        }
    }

    /// The band the half-width is held to, as fractions of the mid.
    #[must_use]
    pub fn band(self) -> (Decimal, Decimal) {
        match self {
            Self::Tight => (Decimal::new(2, 2), Decimal::new(29, 2)),
            Self::Normal => (Decimal::new(3, 2), Decimal::new(47, 2)),
            Self::Wide => (Decimal::new(4, 2), Decimal::new(60, 2)),
        }
    }
}

/// Which way the grid leans: flat at the start, or holding the position the far side's levels
/// will sell (long) or buy back (short).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Neutral,
    Long,
    Short,
}

impl Direction {
    pub const NAMES: &'static [&'static str] = &["neutral", "long", "short"];
}

/// The standard deviation (sample) of the close-to-close returns of `candles`; `None` with fewer
/// than three closes or a non-positive one.
#[must_use]
pub fn daily_sigma(candles: &[Candle]) -> Option<f64> {
    let closes: Vec<f64> = candles.iter().map(|c| c.close.to_f64()).collect::<Option<_>>()?;
    if closes.len() < 3 || closes.iter().any(|c| *c <= 0.0) {
        return None;
    }
    let returns: Vec<f64> = closes.windows(2).map(|w| w[1] / w[0] - 1.0).collect();
    #[expect(clippy::cast_precision_loss, reason = "a few hundred returns at most")]
    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;
    let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
    Some(var.sqrt())
}

/// The average true range of the last [`ATR_PERIOD`] candles (each against the close before it);
/// `None` without `ATR_PERIOD + 1` candles.
#[must_use]
pub fn atr(candles: &[Candle]) -> Option<Decimal> {
    if candles.len() < ATR_PERIOD + 1 {
        return None;
    }
    let tail = &candles[candles.len() - ATR_PERIOD - 1..];
    let sum: Decimal = tail
        .windows(2)
        .map(|w| {
            let (prev, c) = (w[0].close, w[1]);
            (c.high - c.low).max((c.high - prev).abs()).max((c.low - prev).abs())
        })
        .sum();
    Some(sum / Decimal::from(ATR_PERIOD as u64))
}

fn up_to(value: Decimal, tick: Decimal) -> Decimal {
    (value / tick).ceil() * tick
}

fn to_tick(value: Decimal, tick: Decimal) -> Decimal {
    (value / tick).round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven) * tick
}

/// The range around `mid` for `preset` from σ: `(lower, upper)` on the tick.
#[must_use]
pub fn range(mid: Decimal, sigma: f64, preset: RangePreset, tick: Decimal) -> (Decimal, Decimal) {
    let (lo, hi) = preset.band();
    let h = Decimal::from_f64(3.0 * sigma).unwrap_or(hi).clamp(lo, hi);
    (to_tick(mid * (Decimal::ONE - h), tick), to_tick(mid * (Decimal::ONE + h), tick))
}

/// The least gap between levels: 0.77 ATR, 5 ticks or 0.1 % of the mid, whichever is widest,
/// up to the tick.
#[must_use]
pub fn step(atr: Decimal, tick: Decimal, mid: Decimal) -> Decimal {
    let floor =
        (Decimal::new(77, 2) * atr).max(tick * Decimal::from(5)).max(mid / Decimal::from(1000));
    up_to(floor, tick)
}

/// Levels across `[lower, upper]` at least `step` apart, at most `cap` (the dollars' and
/// [`MAX_LEVELS`]).
#[must_use]
pub fn levels(lower: Decimal, upper: Decimal, step: Decimal, cap: usize) -> usize {
    let by_range = ((upper - lower) / step).floor().to_usize().unwrap_or(0).saturating_add(1);
    by_range.min(cap).min(MAX_LEVELS)
}

/// How many levels the dollars can hold at one lot each at `mid` (the venue's least order),
/// with the empty level at the mid.
#[must_use]
pub fn levels_for_dollars(dollars: Decimal, lot: Decimal, mid: Decimal) -> usize {
    let one = lot * mid;
    if one <= Decimal::ZERO {
        return 0;
    }
    (dollars / one).floor().to_usize().unwrap_or(usize::MAX).saturating_add(1)
}

#[cfg(test)]
#[path = "derive_tests.rs"]
mod tests;
