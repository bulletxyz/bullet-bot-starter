use super::*;

fn bounds() -> SpreadBounds {
    SpreadBounds { min_half_spread_bps: 5.0, max_half_spread_bps: 500.0 }
}

#[test]
fn neutral_inventory_quotes_symmetric() {
    let q = quote(100.0, 0.0, 0.5, 1.5, 0.01, 60.0, bounds());
    assert!((q.reservation_price - 100.0).abs() < 1e-9);
    assert!((q.bid + q.ask - 200.0).abs() < 1e-9, "should be symmetric around 100");
    assert!(q.bid < 100.0 && q.ask > 100.0);
}

#[test]
fn long_inventory_skews_quotes_down() {
    let neutral = quote(100.0, 0.0, 0.5, 1.5, 0.01, 60.0, bounds());
    let long = quote(100.0, 0.5, 0.5, 1.5, 0.01, 60.0, bounds());
    // Reservation price should shift below mid when long.
    assert!(long.reservation_price < neutral.reservation_price);
    // Both quotes should shift down — buy side gets more selective, sell
    // side gets easier to hit, naturally unwinding inventory.
    assert!(long.bid < neutral.bid);
    assert!(long.ask < neutral.ask);
}

#[test]
fn short_inventory_skews_quotes_up() {
    let neutral = quote(100.0, 0.0, 0.5, 1.5, 0.01, 60.0, bounds());
    let short = quote(100.0, -0.5, 0.5, 1.5, 0.01, 60.0, bounds());
    assert!(short.reservation_price > neutral.reservation_price);
    assert!(short.bid > neutral.bid);
    assert!(short.ask > neutral.ask);
}

#[test]
fn higher_gamma_increases_skew_magnitude() {
    let low = quote(100.0, 0.5, 0.1, 1.5, 0.01, 60.0, bounds());
    let high = quote(100.0, 0.5, 2.0, 1.5, 0.01, 60.0, bounds());
    // Higher γ → reservation moves further below mid under same inventory.
    assert!(high.reservation_price < low.reservation_price);
}

#[test]
fn low_vol_gets_floored_by_min_spread() {
    // Choose params where both A-S terms are tiny so the raw half-spread
    // is well below the floor. γ=10, κ=1000 → (2/γ)ln(1+γ/κ) ≈ 0.002; σ
    // tiny and τ tiny → γσ²τ negligible.
    let q = quote(100.0, 0.0, 10.0, 1000.0, 1e-6, 1.0, bounds());
    let min_half = 100.0 * 5.0 / 10_000.0;
    assert!(
        (q.half_spread - min_half).abs() < 1e-9,
        "half_spread {} should equal min {}",
        q.half_spread,
        min_half
    );
}

#[test]
fn high_vol_gets_capped_by_max_spread() {
    // Huge σ and long horizon blow up A-S; ceiling must clamp.
    let q = quote(100.0, 0.0, 0.5, 1.5, 5.0, 600.0, bounds());
    let max_half = 100.0 * 500.0 / 10_000.0;
    assert!(
        (q.half_spread - max_half).abs() < 1e-9,
        "half_spread {} should equal max {}",
        q.half_spread,
        max_half
    );
}

#[test]
fn clamps_preserve_bid_leq_mid_leq_ask() {
    // Extreme long inventory — reservation pushed well below mid.
    let q = quote(100.0, 1.0, 2.0, 1.5, 0.1, 600.0, bounds());
    assert!(q.bid <= 100.0, "bid {} should not cross above mid", q.bid);
    assert!(q.ask >= 100.0, "ask {} should not cross below mid", q.ask);
}

#[test]
fn heavy_short_inventory_keeps_ask_wide() {
    // Heavy short → reservation well above mid, so the raw bid would be
    // above mid. We pin bid = mid - min_half, but the ask (well above
    // mid) should be preserved unchanged — wide ask is the signal that
    // discourages more shorting.
    let q = quote(100.0, -1.0, 2.0, 1.5, 0.1, 600.0, bounds());
    assert!(q.bid <= 100.0);
    // Ask should be materially above mid + min_half, not collapsed to it.
    let min_half = 100.0 * 5.0 / 10_000.0;
    assert!(
        q.ask > 100.0 + min_half + 1e-6,
        "ask {} should preserve the A-S skew (> mid + min_half = {})",
        q.ask,
        100.0 + min_half
    );
}

#[test]
fn heavy_long_inventory_keeps_bid_wide() {
    // Symmetric case: heavy long → ask would fall below mid. Pin ask;
    // keep the (wide, below-mid) bid that discourages adding to the long.
    let q = quote(100.0, 1.0, 2.0, 1.5, 0.1, 600.0, bounds());
    assert!(q.ask >= 100.0);
    let min_half = 100.0 * 5.0 / 10_000.0;
    assert!(
        q.bid < 100.0 - min_half - 1e-6,
        "bid {} should preserve the A-S skew (< mid - min_half = {})",
        q.bid,
        100.0 - min_half
    );
}

#[test]
fn ladder_steps_outward_and_preserves_inner() {
    let (inner, rungs) = quote_ladder(100.0, 0.0, 0.5, 1.5, 0.01, 60.0, bounds(), 3, 10.0);
    assert_eq!(rungs.len(), 3);
    // Level 0 matches the inner A-S quote exactly.
    assert!((rungs[0].bid - inner.bid).abs() < 1e-12);
    assert!((rungs[0].ask - inner.ask).abs() < 1e-12);
    // Each outer level is 10 bps of mid = 0.1 further out than the prior.
    assert!((rungs[1].half_spread - rungs[0].half_spread - 0.1).abs() < 1e-9);
    assert!((rungs[2].half_spread - rungs[1].half_spread - 0.1).abs() < 1e-9);
    // Bids strictly decrease, asks strictly increase with level.
    assert!(rungs[1].bid < rungs[0].bid && rungs[2].bid < rungs[1].bid);
    assert!(rungs[1].ask > rungs[0].ask && rungs[2].ask > rungs[1].ask);
}

#[test]
fn ladder_single_level_matches_inner() {
    let (inner, rungs) = quote_ladder(100.0, 0.3, 0.5, 1.5, 0.01, 60.0, bounds(), 1, 10.0);
    assert_eq!(rungs.len(), 1);
    assert!((rungs[0].bid - inner.bid).abs() < 1e-12);
    assert!((rungs[0].ask - inner.ask).abs() < 1e-12);
}

#[test]
fn kappa_tightens_base_spread() {
    // Increasing κ (more competitive market) shrinks the (2/γ)·ln(1+γ/κ) term.
    // Use small τ so the γσ²τ term doesn't dominate, and disable clamps.
    let loose_bounds = SpreadBounds { min_half_spread_bps: 0.0, max_half_spread_bps: 10_000.0 };
    let low_k = quote(100.0, 0.0, 0.5, 0.5, 0.01, 1.0, loose_bounds);
    let high_k = quote(100.0, 0.0, 0.5, 50.0, 0.01, 1.0, loose_bounds);
    assert!(high_k.half_spread < low_k.half_spread);
}
