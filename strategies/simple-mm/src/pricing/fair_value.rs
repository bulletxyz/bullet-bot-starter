//! Fair value for quoting: the Binance USD-M perp mid plus a slow EMA of the basis (Bullet mid
//! minus Binance mid), or the Bullet mid.
//!
//! The basis EMA absorbs USD vs USDT and any steady venue premium, so the fair value tracks Binance
//! moves without drifting from where Bullet trades. When the Binance mid is missing or stale, or
//! the basis has not been averaged for [`BASIS_WARMUP_SECS`], the Bullet mid is used and the
//! reason is reported. Quoting never stops because Binance is down. Times are the bot clock (ms).

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal prices and basis sit far below 7.9e28; mid checked > 0 before dividing"
)]

use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use serde::Serialize;

use crate::params::{FairValue, MmParams};

/// Seconds of continuous basis averaging before the Binance fair value is used.
pub const BASIS_WARMUP_SECS: u64 = 60;
/// Seconds of continuous divergence after which the basis is re-learned.
pub const BASIS_RELEARN_SECS: u64 = 300;
/// Decimal places kept in the basis EMA (keeps quote prices tidy).
const BASIS_DP: u32 = 8;

const fn ms(secs: u64) -> u64 {
    secs.saturating_mul(1000)
}

/// The fair value quotes are centred on, and where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fair {
    pub price: Decimal,
    pub source: FairValue,
    /// Set when `binance` is configured but the Bullet mid is used.
    pub fallback: Option<Fallback>,
}

/// Why the Bullet mid is used although `fair_value = "binance"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Fallback {
    /// No Binance mid received yet.
    NoReference,
    /// No Binance mid for more than `reference_stale_secs`.
    ReferenceStale { secs: u64 },
    /// The basis has been averaged for less than [`BASIS_WARMUP_SECS`].
    BasisWarming,
    /// The Binance fair value is more than `max_divergence_bps` from the Bullet mid.
    Diverged { bps: Decimal },
}

impl std::fmt::Display for Fallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoReference => f.write_str("no Binance mid received yet"),
            Self::ReferenceStale { secs } => {
                write!(f, "Binance mid stale (no update for over {secs}s)")
            }
            Self::BasisWarming => write!(
                f,
                "Bullet-Binance basis still averaging (needs {BASIS_WARMUP_SECS}s of Binance data)"
            ),
            Self::Diverged { bps } => {
                write!(f, "Binance fair value diverges from Bullet by {bps} bps")
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Basis {
    pub(crate) ema: Decimal,
    updated: u64,
    since: u64,
    /// Start of the current run of divergent (left out) samples.
    diverged_since: Option<u64>,
}

/// Tracks the Binance mid and the basis EMA.
#[derive(Debug, Default)]
pub struct FairValueTracker {
    /// Last Binance mid and when it arrived (the feed's receive time, so a backlogged update is
    /// judged by its real age).
    pub(crate) reference: Option<(Decimal, u64)>,
    pub(crate) basis: Option<Basis>,
}

impl FairValueTracker {
    /// A Binance best bid / ask, received at `received_ms`.
    pub fn on_reference(&mut self, bid: Decimal, ask: Decimal, received_ms: u64) {
        let newer = self.reference.is_none_or(|(_, at)| received_ms >= at);
        if bid > Decimal::ZERO && ask >= bid && newer {
            self.reference = Some(((bid + ask) / Decimal::TWO, received_ms));
        }
    }

    fn fresh_reference(&self, now_ms: u64, stale_secs: u64) -> Result<Decimal, Fallback> {
        match self.reference {
            None => Err(Fallback::NoReference),
            Some((_, at)) if now_ms.saturating_sub(at) > ms(stale_secs) => {
                Err(Fallback::ReferenceStale { secs: stale_secs })
            }
            Some((mid, _)) => Ok(mid),
        }
    }

    /// A fresh Bullet mid: fold `mid - binance_mid` into the basis. Returns the shift in bps when
    /// a sustained divergence restarts the warm-up. Without a fresh Binance mid the basis is
    /// dropped, so it warms up again. Each sample gets weight `max(EMA weight, dt / time since
    /// the first sample)`: early on that is the time-weighted mean of the samples so far, later
    /// the half-life EMA.
    pub fn on_bullet_mid(
        &mut self,
        mid: Decimal,
        now_ms: u64,
        params: &MmParams,
    ) -> Option<Decimal> {
        let Ok(reference) = self.fresh_reference(now_ms, params.reference_stale_secs) else {
            self.basis = None;
            return None;
        };
        let sample = mid - reference;
        let seed = Basis { ema: sample, updated: now_ms, since: now_ms, diverged_since: None };
        let Some(b) = self.basis else {
            self.basis = Some(seed);
            return None;
        };
        let warm = now_ms.saturating_sub(b.since) >= ms(BASIS_WARMUP_SECS);
        let off_bps = if mid > Decimal::ZERO {
            (sample - b.ema) / mid * Decimal::from(10_000)
        } else {
            Decimal::ZERO
        };
        // Once warm, a sample that would put the fair value more than `max_divergence_bps` from
        // the Bullet mid is left out, so a divergent Binance does not drag the basis after it. If
        // that lasts `BASIS_RELEARN_SECS`, the basis has really moved: warm up again from it.
        if warm && off_bps.abs() > params.max_divergence_bps {
            let since = b.diverged_since.unwrap_or(now_ms);
            if now_ms.saturating_sub(since) >= ms(BASIS_RELEARN_SECS) {
                self.basis = Some(seed);
                return Some(off_bps.round_dp(1));
            }
            self.basis = Some(Basis { diverged_since: Some(since), ..b });
            return None;
        }
        let alpha = ema_alpha(
            now_ms.saturating_sub(b.updated),
            now_ms.saturating_sub(b.since),
            params.basis_halflife_secs,
        );
        self.basis = Some(Basis {
            ema: (b.ema + alpha * (sample - b.ema)).round_dp(BASIS_DP),
            updated: now_ms,
            since: b.since,
            diverged_since: None,
        });
        None
    }

    /// The fair value at `now_ms` given the Bullet mid.
    #[must_use]
    pub fn fair(&self, bullet_mid: Decimal, now_ms: u64, params: &MmParams) -> Fair {
        let bullet = |fallback| Fair { price: bullet_mid, source: FairValue::Bullet, fallback };
        if params.fair_value == FairValue::Bullet {
            return bullet(None);
        }
        let reference = match self.fresh_reference(now_ms, params.reference_stale_secs) {
            Ok(r) => r,
            Err(why) => return bullet(Some(why)),
        };
        let price = match self.basis {
            Some(b) if now_ms.saturating_sub(b.since) >= ms(BASIS_WARMUP_SECS) => reference + b.ema,
            _ => return bullet(Some(Fallback::BasisWarming)),
        };
        if bullet_mid > Decimal::ZERO {
            let bps = ((price - bullet_mid).abs() / bullet_mid * Decimal::from(10_000)).round_dp(1);
            if bps > params.max_divergence_bps {
                return bullet(Some(Fallback::Diverged { bps }));
            }
        }
        Fair { price, source: FairValue::Binance, fallback: None }
    }

    /// The basis EMA in bps of `bullet_mid` (2 dp), once there is one.
    #[must_use]
    pub fn basis_bps(&self, bullet_mid: Decimal) -> Option<Decimal> {
        let b = self.basis?;
        (bullet_mid > Decimal::ZERO)
            .then(|| (b.ema / bullet_mid * Decimal::from(10_000)).round_dp(2))
    }
}

/// Weight of a sample `dt_ms` after the previous one, `elapsed_ms` after the first:
/// `max(1 - 2^(-dt/half_life), dt/elapsed)`. A sample at the same instant as the previous one
/// weighs nothing.
#[expect(clippy::cast_precision_loss, reason = "millisecond spans, far below 2^53")]
fn ema_alpha(dt_ms: u64, elapsed_ms: u64, halflife_secs: u64) -> Decimal {
    if dt_ms == 0 {
        return Decimal::ZERO;
    }
    let half_life = ms(halflife_secs.max(1)) as f64;
    let ema = 1.0 - 0.5_f64.powf(dt_ms as f64 / half_life);
    // elapsed >= dt > 0.
    let mean = dt_ms as f64 / elapsed_ms as f64;
    Decimal::from_f64(ema.max(mean).clamp(0.0, 1.0)).unwrap_or(Decimal::ONE)
}

#[cfg(test)]
#[path = "fair_value_tests.rs"]
mod tests;
