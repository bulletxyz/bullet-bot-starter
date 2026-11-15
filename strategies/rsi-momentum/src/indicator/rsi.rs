//! Wilder's RSI over candle closes. Pure.
//!
//! The first value needs `period + 1` closes (`period` changes): the averages start as the simple
//! mean of the first `period` gains and losses, then smooth as
//! `avg = (avg x (period - 1) + change) / period`. `RSI = 100 - 100 / (1 + avg_gain / avg_loss)`;
//! no losses reads 100, neither gains nor losses 50.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal prices far below 7.9e28; the period divisor is validated >= 2; the change counter is bounded by the period"
)]

use rust_decimal::Decimal;

#[derive(Debug, Clone)]
pub struct Rsi {
    period: u32,
    prev_close: Option<Decimal>,
    /// Changes summed while seeding, up to `period`.
    seeded: u32,
    sum_gain: Decimal,
    sum_loss: Decimal,
    /// `(avg_gain, avg_loss)` once seeded.
    avg: Option<(Decimal, Decimal)>,
}

const HUNDRED: Decimal = Decimal::ONE_HUNDRED;
const FIFTY: Decimal = Decimal::from_parts(50, 0, 0, false, 0);

fn value(avg_gain: Decimal, avg_loss: Decimal) -> Decimal {
    if avg_loss.is_zero() {
        return if avg_gain.is_zero() { FIFTY } else { HUNDRED };
    }
    // A run with no loss leaves avg_loss a few 1e-28 above zero, where the ratio overflows: 100.
    avg_gain
        .checked_div(avg_loss)
        .and_then(|ratio| ratio.checked_add(Decimal::ONE))
        .map_or(HUNDRED, |d| HUNDRED - HUNDRED / d)
}

impl Rsi {
    /// `period` must be at least 1 (the config holds it to 2..=100).
    #[must_use]
    pub fn new(period: u32) -> Self {
        Self {
            period: period.max(1),
            prev_close: None,
            seeded: 0,
            sum_gain: Decimal::ZERO,
            sum_loss: Decimal::ZERO,
            avg: None,
        }
    }

    /// Forget every close: the next value needs `period + 1` closes again.
    pub fn reset(&mut self) {
        *self = Self::new(self.period);
    }

    /// The closes still needed before the first value (0 once warm).
    #[must_use]
    pub fn closes_needed(&self) -> u32 {
        if self.avg.is_some() {
            return 0;
        }
        let have = self.seeded + u32::from(self.prev_close.is_some());
        (self.period + 1).saturating_sub(have)
    }

    /// Add a close; the RSI after it, `None` while warming up.
    pub fn push(&mut self, close: Decimal) -> Option<Decimal> {
        let prev = self.prev_close.replace(close)?;
        let change = close - prev;
        let (gain, loss) =
            if change > Decimal::ZERO { (change, Decimal::ZERO) } else { (Decimal::ZERO, -change) };
        let n = Decimal::from(self.period);
        let (g, l) = if let Some((g, l)) = self.avg {
            ((g * (n - Decimal::ONE) + gain) / n, (l * (n - Decimal::ONE) + loss) / n)
        } else {
            self.sum_gain += gain;
            self.sum_loss += loss;
            self.seeded += 1;
            if self.seeded < self.period {
                return None;
            }
            (self.sum_gain / n, self.sum_loss / n)
        };
        self.avg = Some((g, l));
        Some(value(g, l))
    }
}

#[cfg(test)]
#[path = "rsi_tests.rs"]
pub(crate) mod tests;
