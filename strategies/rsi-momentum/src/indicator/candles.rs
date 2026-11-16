//! OHLC candles built from a price stream, closed on interval boundaries by the event's
//! timestamp. Pure: the actor feeds it each mark with the clock's time (the replayed time under
//! a test clock), so live, paper and replay build the same candles.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "interval lengths are fixed nonzero constants; bucket math on u64 ms timestamps stays far inside u64"
)]

use bb_strategy_sdk::{Candle, CandleInterval};
use rust_decimal::Decimal;

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
    interval: CandleInterval,
    current: Option<Candle>,
}

impl CandleBuilder {
    #[must_use]
    pub fn new(interval: CandleInterval) -> Self {
        Self { interval, current: None }
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
        let bucket = self.interval.open_of(ts_ms);
        let fresh = Candle {
            open_ms: bucket,
            open: price,
            high: price,
            low: price,
            close: price,
            volume: Decimal::ZERO,
        };
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
        let missed = (bucket - cur.open_ms) / self.interval.ms() - 1;
        let candle = *cur;
        self.current = Some(fresh);
        Some(Closed { candle, missed })
    }
}

#[cfg(test)]
#[path = "candles_tests.rs"]
mod tests;
