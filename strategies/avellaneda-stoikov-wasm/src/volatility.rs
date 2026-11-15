//! The rolling-window volatility estimator, the built-in's, with its samples stamped in the bot's
//! milliseconds (`View::now_ms`) instead of the OS clock.

use std::collections::VecDeque;

/// Stores `(t, ln(mid))` samples for the last `window` and exposes the standard deviation of
/// inter-sample log returns as an estimate of volatility.
///
/// The returned `sigma` is the stddev of log returns per sample step. Paired with a `tau_secs`
/// horizon in the A-S formula, `sigma² * tau` is "variance of fractional mid change over the recent
/// past" and `tau` a tuning knob: nothing is annualised.
#[derive(Debug, Clone)]
pub struct Volatility {
    window_ms: u64,
    samples: VecDeque<(u64, f64)>,
}

impl Volatility {
    #[must_use]
    pub fn new(window_secs: u64) -> Self {
        Self { window_ms: window_secs.max(1).saturating_mul(1000), samples: VecDeque::new() }
    }

    /// Push a fresh mid observation at `now_ms` and evict anything older than the window.
    pub fn push(&mut self, mid: f64, now_ms: u64) {
        if mid <= 0.0 || !mid.is_finite() {
            return;
        }
        self.samples.push_back((now_ms, mid.ln()));
        let cutoff = now_ms.saturating_sub(self.window_ms);
        while let Some((t, _)) = self.samples.front() {
            if *t < cutoff {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }

    /// Stddev of log returns across the buffered samples. `None` until there are at least three
    /// samples (two returns are needed for the Bessel-corrected sample variance).
    #[must_use]
    #[expect(clippy::cast_precision_loss, reason = "sample counts are tiny")]
    pub fn sigma(&self) -> Option<f64> {
        if self.samples.len() < 2 {
            return None;
        }
        let returns: Vec<f64> = self
            .samples
            .iter()
            .zip(self.samples.iter().skip(1))
            .map(|((_, a), (_, b))| b - a)
            .collect();
        let n = returns.len() as f64;
        if n < 2.0 {
            return None;
        }
        let mean = returns.iter().sum::<f64>() / n;
        // Bessel's correction: without it sigma is systematically too tight at low sample counts.
        let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
        Some(var.sqrt())
    }

    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }
}

#[cfg(test)]
#[path = "volatility_tests.rs"]
mod tests;
