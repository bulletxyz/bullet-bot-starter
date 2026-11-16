use std::collections::VecDeque;

/// Rolling-window volatility estimator. Stores `(t, ln(mid))` samples for the
/// last `window` duration and exposes the standard deviation of inter-sample
/// log returns as an estimate of per-second price volatility.
///
/// Units: the returned `sigma` is the stddev of log returns per sample step.
/// When paired with a `tau_secs` horizon in the A-S formula, the caller
/// should scale `sigma² * tau` consistently. The strategy code below treats
/// `sigma²` as "variance of fractional mid change over the recent past" and
/// `tau` as a pure tuning knob — we don't try to annualize.
#[derive(Debug, Clone)]
pub struct Volatility {
    window_ms: u64,
    samples: VecDeque<(u64, f64)>,
}

impl Volatility {
    pub fn new(window_secs: u64) -> Self {
        Self { window_ms: window_secs.max(1).saturating_mul(1000), samples: VecDeque::new() }
    }

    /// Push a fresh mid observation (`now_ms` on the bot clock) and evict anything older than the
    /// window.
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

    /// Stddev of log returns across the buffered samples. Returns `None` until
    /// we have at least three samples (two returns needed for Bessel-corrected
    /// sample variance; one return gives 0/0 which is undefined).
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
            // Single return → no variance estimate (0 degrees of freedom).
            return None;
        }
        let mean = returns.iter().sum::<f64>() / n;
        // Bessel's correction (÷ n−1) gives the unbiased sample variance.
        // Without it, sigma is systematically too tight at low sample counts,
        // which causes the A-S spread to be narrower than the model requires.
        let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
        Some(var.sqrt())
    }

    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }
}

#[cfg(test)]
#[expect(clippy::cast_precision_loss, reason = "test sample counts are tiny")]
#[path = "volatility_tests.rs"]
mod tests;
