//! OHLC candles built from a price stream, closed on interval boundaries by the event's
//! timestamp. Pure: the actor feeds it each mark with the clock's time (the replayed time under
//! a test clock), so live, paper and replay build the same candles.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "interval lengths are fixed nonzero constants; bucket math on u64 ms timestamps stays far inside u64"
)]

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// A candle length. The set Bullet's kline topic offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
pub enum Interval {
    #[serde(rename = "1m")]
    M1,
    #[serde(rename = "5m")]
    M5,
    #[serde(rename = "15m")]
    M15,
    #[serde(rename = "30m")]
    M30,
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "4h")]
    H4,
    #[serde(rename = "1d")]
    D1,
}

impl Interval {
    /// The config names, in declaration order (the descriptor's choices).
    pub const NAMES: &'static [&'static str] = &["1m", "5m", "15m", "30m", "1h", "4h", "1d"];

    /// Length in milliseconds.
    #[must_use]
    pub const fn ms(self) -> u64 {
        const MIN: u64 = 60_000;
        match self {
            Self::M1 => MIN,
            Self::M5 => 5 * MIN,
            Self::M15 => 15 * MIN,
            Self::M30 => 30 * MIN,
            Self::H1 => 60 * MIN,
            Self::H4 => 240 * MIN,
            Self::D1 => 1_440 * MIN,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::M1 => "1m",
            Self::M5 => "5m",
            Self::M15 => "15m",
            Self::M30 => "30m",
            Self::H1 => "1h",
            Self::H4 => "4h",
            Self::D1 => "1d",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candle {
    /// Start of the bucket, Unix ms (a multiple of the interval).
    pub open_ms: u64,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
}

/// A candle that just closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed {
    pub candle: Candle,
    /// Whole buckets with no price between this candle and the price that closed it. Nonzero is
    /// a hole in the data: the indicator should not bridge it.
    pub missed: u64,
}

/// Builds candles from `(timestamp, price)`.
#[derive(Debug, Clone)]
pub struct CandleBuilder {
    interval_ms: u64,
    current: Option<Candle>,
}

impl CandleBuilder {
    #[must_use]
    pub fn new(interval: Interval) -> Self {
        Self { interval_ms: interval.ms(), current: None }
    }

    /// The candle being built.
    #[must_use]
    pub fn current(&self) -> Option<&Candle> {
        self.current.as_ref()
    }

    /// Feed one price at `ts_ms`. Returns the candle it closed: the first price in a later bucket
    /// closes the one in progress. A non-positive price, or one stamped in a bucket before the
    /// current one (a clock that stepped back), is ignored.
    pub fn on_price(&mut self, ts_ms: u64, price: Decimal) -> Option<Closed> {
        if price <= Decimal::ZERO {
            return None;
        }
        let bucket = ts_ms - ts_ms % self.interval_ms;
        let fresh = Candle { open_ms: bucket, open: price, high: price, low: price, close: price };
        let Some(cur) = self.current.as_mut() else {
            self.current = Some(fresh);
            return None;
        };
        if bucket < cur.open_ms {
            return None;
        }
        if bucket == cur.open_ms {
            cur.high = cur.high.max(price);
            cur.low = cur.low.min(price);
            cur.close = price;
            return None;
        }
        let missed = (bucket - cur.open_ms) / self.interval_ms - 1;
        let candle = *cur;
        self.current = Some(fresh);
        Some(Closed { candle, missed })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: u64 = 60_000;
    /// A UTC day boundary (so every interval's boundary).
    const T0: u64 = 20_000 * 1_440 * M;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn interval_lengths() {
        assert_eq!(Interval::M1.ms(), M);
        assert_eq!(Interval::M15.ms(), 15 * M);
        assert_eq!(Interval::H4.ms(), 4 * 60 * M);
        assert_eq!(Interval::D1.ms(), 24 * 60 * M);
        for (i, name) in Interval::NAMES.iter().enumerate() {
            let parsed: Interval = serde_json::from_value(serde_json::json!(name)).unwrap();
            assert_eq!(parsed.name(), *name, "{i}");
        }
    }

    #[test]
    fn prices_inside_a_bucket_build_one_candle() {
        let mut b = CandleBuilder::new(Interval::M1);
        for (t, p) in [(0, "100"), (10_000, "103"), (20_000, "98"), (59_999, "101")] {
            assert_eq!(b.on_price(T0 + t, d(p)), None);
        }
        let c = b.current().unwrap();
        assert_eq!(
            (c.open_ms, c.open, c.high, c.low, c.close),
            (T0, d("100"), d("103"), d("98"), d("101"))
        );
    }

    #[test]
    fn the_first_price_in_the_next_bucket_closes_the_candle_on_the_boundary() {
        let mut b = CandleBuilder::new(Interval::M1);
        b.on_price(T0 + 5_000, d("100"));
        b.on_price(T0 + 59_999, d("102"));
        let closed = b.on_price(T0 + M, d("104")).expect("the boundary closes it");
        assert_eq!(closed.missed, 0);
        assert_eq!(closed.candle.open_ms, T0);
        assert_eq!(
            closed.candle.close,
            d("102"),
            "the close is the last price before the boundary"
        );
        assert_eq!(b.current().unwrap().open_ms, T0 + M);
        assert_eq!(b.current().unwrap().open, d("104"));
    }

    #[test]
    fn buckets_align_to_the_interval_not_to_the_first_price() {
        let mut b = CandleBuilder::new(Interval::M15);
        b.on_price(T0 + 7 * M, d("1"));
        assert_eq!(b.current().unwrap().open_ms % (15 * M), 0);
        // Still the same 15m bucket 7 minutes later; the next one closes it.
        assert_eq!(b.on_price(T0 + 14 * M, d("2")), None);
        assert!(b.on_price(T0 + 15 * M, d("3")).is_some());
    }

    #[test]
    fn a_gap_reports_the_missed_buckets() {
        let mut b = CandleBuilder::new(Interval::M1);
        b.on_price(T0, d("100"));
        let closed = b.on_price(T0 + 4 * M + 1, d("90")).unwrap();
        assert_eq!(closed.missed, 3, "minutes 1, 2 and 3 had no price");
        assert_eq!(closed.candle.close, d("100"));
    }

    #[test]
    fn a_price_from_an_earlier_bucket_or_not_positive_is_ignored() {
        let mut b = CandleBuilder::new(Interval::M1);
        b.on_price(T0 + M, d("100"));
        assert_eq!(b.on_price(T0 + 1_000, d("50")), None, "the clock stepped back");
        assert_eq!(b.on_price(T0 + M + 1, d("0")), None);
        assert_eq!(b.on_price(T0 + M + 2, d("-1")), None);
        let c = b.current().unwrap();
        assert_eq!((c.open_ms, c.low, c.close), (T0 + M, d("100"), d("100")));
    }
}
