//! Wilder's RSI over candle closes. Pure.
//!
//! The first value needs `period + 1` closes (`period` changes): the averages start as the simple
//! mean of the first `period` gains and losses, then smooth as
//! `avg = (avg x (period - 1) + change) / period`. `RSI = 100 - 100 / (1 + avg_gain / avg_loss)`;
//! no losses reads 100, neither gains nor losses 50.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal prices far below 7.9e28; the period divisor is validated >= 2; the change \
              counter is bounded by the period"
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
    HUNDRED - HUNDRED / (Decimal::ONE + avg_gain / avg_loss)
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
pub(crate) mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    /// The `StockCharts` `ChartSchool` RSI worksheet (Wilder's smoothing, 14 periods): its closes
    /// and the RSI column, to two decimals.
    pub(crate) const CLOSES: [&str; 33] = [
        "44.3389", "44.0902", "44.1497", "43.6124", "44.2778", "44.8264", "45.0955", "45.4245",
        "45.8433", "46.0826", "45.8931", "46.0328", "45.6140", "46.2820", "46.2820", "46.0028",
        "46.0328", "46.4116", "46.2222", "45.6439", "46.2122", "46.2521", "45.7137", "46.4515",
        "45.7835", "45.3548", "44.0288", "44.1783", "44.2181", "44.5672", "43.4205", "42.6628",
        "43.1314",
    ];
    pub(crate) const RSI_14: [&str; 19] = [
        "70.53", "66.32", "66.55", "69.41", "66.36", "57.97", "62.93", "63.26", "56.06", "62.38",
        "54.71", "50.42", "39.99", "41.46", "41.87", "45.46", "37.30", "33.08", "37.77",
    ];

    #[test]
    fn matches_the_wilder_reference_series() {
        let mut rsi = Rsi::new(14);
        let out: Vec<Option<Decimal>> = CLOSES.iter().map(|c| rsi.push(d(c))).collect();
        assert!(out[..14].iter().all(Option::is_none), "14 changes before the first value");
        let got: Vec<Decimal> = out[14..].iter().map(|v| v.unwrap().round_dp(2)).collect();
        let want: Vec<Decimal> = RSI_14.iter().map(|v| d(v)).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn the_first_value_needs_period_plus_one_closes() {
        let mut rsi = Rsi::new(3);
        assert_eq!(rsi.closes_needed(), 4);
        for (i, c) in ["1", "2", "3"].iter().enumerate() {
            assert_eq!(rsi.push(d(c)), None, "close {i}");
        }
        assert_eq!(rsi.closes_needed(), 1);
        assert_eq!(rsi.push(d("4")), Some(d("100")), "only gains");
        assert_eq!(rsi.closes_needed(), 0);
    }

    #[test]
    fn edge_values_and_reset() {
        let mut flat = Rsi::new(2);
        for c in ["5", "5", "5"] {
            flat.push(d(c));
        }
        assert_eq!(flat.push(d("5")), Some(d("50")), "no movement");
        let mut down = Rsi::new(2);
        for c in ["5", "4"] {
            down.push(d(c));
        }
        assert_eq!(down.push(d("3")), Some(Decimal::ZERO), "only losses");
        down.reset();
        assert_eq!(down.closes_needed(), 3);
        assert_eq!(down.push(d("1")), None);
    }

    /// Period 2 by hand: +1, +1 -> 100; -3 -> gain 0.5, loss 1.5 -> 25; +4 -> 2.25 / 0.75 -> 75.
    #[test]
    fn wilder_smoothing_by_hand() {
        let mut rsi = Rsi::new(2);
        let out: Vec<Option<Decimal>> = ["100", "101", "102", "99", "103"]
            .iter()
            .map(|c| rsi.push(d(c)).map(|v| v.round_dp(10).normalize()))
            .collect();
        assert_eq!(out, [None, None, Some(d("100")), Some(d("25")), Some(d("75"))]);
    }
}
