//! Avellaneda-Stoikov closed-form quote math.
//!
//! Reference: Avellaneda & Stoikov (2008), *High-frequency trading in a limit
//! order book*. The finite-horizon formulas used here are the ones Hummingbot
//! implements in `avellaneda_market_making`; they extend to the GLFT
//! stationary limit as τ grows.
//!
//! Given mid `s`, inventory `q` (base units, signed), risk aversion `γ > 0`,
//! order-flow intensity `κ > 0`, volatility `σ`, and horizon `τ`:
//!
//! ```text
//! reservation_price = s - q · γ · σ² · τ
//! optimal_spread    = γ · σ² · τ + (2/γ) · ln(1 + γ/κ)
//! bid = reservation_price - optimal_spread / 2
//! ask = reservation_price + optimal_spread / 2
//! ```
//!
//! All inputs and outputs here are `f64`; the strategy layer converts to/from
//! `Decimal` at the boundary.

/// A single pair of quotes produced by the model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quote {
    pub bid: f64,
    pub ask: f64,
    pub reservation_price: f64,
    pub half_spread: f64,
}

/// Bounds enforced on the half-spread after A-S produces a raw value.
#[derive(Debug, Clone, Copy)]
pub struct SpreadBounds {
    pub min_half_spread_bps: f64,
    pub max_half_spread_bps: f64,
}

/// One rung of a multi-level quote ladder.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LadderRung {
    pub level: usize,
    pub bid: f64,
    pub ask: f64,
    pub half_spread: f64,
}

/// Core A-S quote computation.
///
/// `inventory_normalized` should be `(net_position - inventory_target) /
/// max_position`, clipped to [-1, 1]. Normalizing makes γ stable across
/// position sizes — exactly what Hummingbot's `inventory_target_base_pct`
/// achieves by a different spelling.
pub fn quote(
    mid: f64,
    inventory_normalized: f64,
    gamma: f64,
    kappa: f64,
    sigma: f64,
    tau_secs: f64,
    bounds: SpreadBounds,
) -> Quote {
    debug_assert!(gamma > 0.0 && kappa > 0.0, "gamma and kappa must be > 0");
    let q = inventory_normalized.clamp(-1.0, 1.0);
    let sigma2 = sigma * sigma;

    // Reservation price: inventory drags it away from mid.
    let reservation_price = mid - q * gamma * sigma2 * tau_secs;

    // A-S optimal *total* spread. Second term dominates when σ is small; first
    // term widens in vol or long horizons.
    let raw_spread = gamma * sigma2 * tau_secs + (2.0 / gamma) * (1.0 + gamma / kappa).ln();
    let raw_half = raw_spread / 2.0;

    // Clamp to configured bounds (in bps of mid, converted to price terms).
    let min_half = mid * bounds.min_half_spread_bps / 10_000.0;
    let max_half = mid * bounds.max_half_spread_bps / 10_000.0;
    let half_spread = raw_half.clamp(min_half, max_half);

    let bid = reservation_price - half_spread;
    let ask = reservation_price + half_spread;

    // Post-clamp sanity: ensure bid ≤ mid ≤ ask. Heavy inventory can shift
    // the reservation price far enough that bid > mid (heavy short) or
    // ask < mid (heavy long). Pin only the offending side to mid ± min_half
    // and keep the A-S-optimal quote on the other side — that preserves the
    // inventory-unwind skew (wide ask when short / wide bid when long) while
    // respecting PostOnly. The `ask` in the `bid > mid` branch is already
    // ≥ mid + min_half because `half ≥ min_half`, so keeping it is safe.
    let (bid, ask) = if bid > mid {
        (mid - min_half, ask)
    } else if ask < mid {
        (bid, mid + min_half)
    } else {
        (bid, ask)
    };

    Quote { bid, ask, reservation_price, half_spread }
}

/// Multi-level quote ladder. Level 0 is the A-S inner quote (output of
/// [`quote`]); each outer level steps out by `level_spread_bps × mid / 10_000`
/// from the inner half-spread, anchored to the same reservation price. This
/// is the layout Hummingbot's `avellaneda_market_making` uses via
/// `order_levels` / `order_level_spread`.
// Math API — named scalars beat a struct for the call-site intuition of the
// formula. One extra parameter beyond clippy's soft limit is fine.
#[expect(
    clippy::too_many_arguments,
    clippy::cast_precision_loss,
    reason = "the A-S formula takes each model parameter; rung counts are tiny"
)]
pub fn quote_ladder(
    mid: f64,
    inventory_normalized: f64,
    gamma: f64,
    kappa: f64,
    sigma: f64,
    tau_secs: f64,
    bounds: SpreadBounds,
    levels: usize,
    level_spread_bps: f64,
) -> (Quote, Vec<LadderRung>) {
    let inner = quote(mid, inventory_normalized, gamma, kappa, sigma, tau_secs, bounds);
    let step = mid * level_spread_bps / 10_000.0;
    let n = levels.max(1);
    let mut rungs = Vec::with_capacity(n);
    for i in 0..n {
        let half = inner.half_spread + (i as f64) * step;
        rungs.push(LadderRung {
            level: i,
            bid: inner.reservation_price - half,
            ask: inner.reservation_price + half,
            half_spread: half,
        });
    }
    (inner, rungs)
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
