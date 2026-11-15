/// Time-weighted exponential moving average.
///
/// `tau_secs` is the time constant: a step input decays to `1/e` of its
/// terminal value after `tau_secs`. Larger `tau_secs` → smoother, slower.
#[derive(Debug, Clone)]
pub struct Ema {
    tau_secs: f64,
    value: Option<f64>,
    /// The bot's clock (unix ms) at the last sample.
    last_update_ms: Option<u64>,
}

impl Ema {
    pub fn new(tau_secs: f64) -> Self {
        Self { tau_secs, value: None, last_update_ms: None }
    }

    /// Fold `sample` in; `now_ms` is the bot's clock (`View::now_ms`), never the wall clock.
    #[expect(clippy::cast_precision_loss, reason = "elapsed ms are far below 2^52")]
    pub fn update(&mut self, sample: f64, now_ms: u64) -> f64 {
        let next = match (self.value, self.last_update_ms) {
            (Some(prev), Some(ts)) => {
                let dt = now_ms.saturating_sub(ts) as f64 / 1000.0;
                let alpha = 1.0 - (-dt / self.tau_secs).exp();
                prev + alpha * (sample - prev)
            }
            _ => sample,
        };
        self.value = Some(next);
        self.last_update_ms = Some(now_ms);
        next
    }

    pub fn value(&self) -> Option<f64> {
        self.value
    }
}

#[cfg(test)]
#[expect(clippy::cast_precision_loss, reason = "test durations are tiny")]
#[path = "ema_tests.rs"]
mod tests;
