use super::*;

const T0: u64 = 1_794_398_400_000;

#[test]
fn seed_and_converge() {
    let mut ema = Ema::new(60.0);
    assert_eq!(ema.update(100.0, T0), 100.0);

    // Step up to 200; after 1 tau (~60s) we're ~63% of the way.
    let v = ema.update(200.0, T0 + 60_000);
    assert!(v > 160.0 && v < 165.0, "expected ~163, got {v}");

    // After a long time relative to tau, should be very close to 200.
    let v = ema.update(200.0, T0 + 600_000);
    assert!((v - 200.0).abs() < 0.5, "expected ~200, got {v}");
}

#[test]
fn fast_vs_slow_divergence() {
    // Fast EMA leads slow EMA on a trending input.
    let mut fast = Ema::new(30.0);
    let mut slow = Ema::new(600.0);

    // Seed both at 100, then drift up by 1/sec for 60 seconds.
    fast.update(100.0, T0);
    slow.update(100.0, T0);
    for i in 1..=60_u64 {
        let t = T0 + i * 1000;
        fast.update(100.0 + i as f64, t);
        slow.update(100.0 + i as f64, t);
    }
    let f = fast.value().unwrap();
    let s = slow.value().unwrap();
    assert!(f > s, "fast ({f}) should lead slow ({s}) on uptrend");
}
