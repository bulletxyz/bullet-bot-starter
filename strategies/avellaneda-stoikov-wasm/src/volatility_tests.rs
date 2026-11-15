use super::*;

#[test]
fn insufficient_samples() {
    let mut v = Volatility::new(60);
    assert_eq!(v.sigma(), None);
    v.push(100.0, 0);
    assert_eq!(v.sigma(), None);
    // Two samples give one return: not enough for an unbiased sample variance.
    v.push(101.0, 1);
    assert_eq!(v.sigma(), None);
    v.push(102.0, 2);
    assert!(v.sigma().is_some());
}

#[test]
fn flat_price_zero_vol() {
    let mut v = Volatility::new(60);
    for i in 0..20 {
        v.push(100.0, i * 100);
    }
    let s = v.sigma().unwrap();
    assert!(s < 1e-9, "flat price should have ~0 vol, got {s}");
}

#[test]
fn trending_price_positive_vol() {
    let mut v = Volatility::new(60);
    for i in 0..20_u32 {
        v.push(100.0 + f64::from(i) * 0.1, u64::from(i) * 100);
    }
    let s = v.sigma().unwrap();
    assert!(s > 0.0 && s.is_finite(), "expected positive finite vol, got {s}");
}

#[test]
fn evicts_old_samples() {
    let mut v = Volatility::new(1);
    for i in 0..5_u32 {
        v.push(100.0 + f64::from(i), u64::from(i) * 100);
    }
    assert_eq!(v.sample_count(), 5);
    v.push(200.0, 10_000);
    assert_eq!(v.sample_count(), 1, "only the newest sample is inside the window");
}

#[test]
fn a_sample_exactly_a_window_old_stays() {
    let mut v = Volatility::new(1);
    v.push(100.0, 1_000);
    v.push(101.0, 2_000);
    assert_eq!(v.sample_count(), 2, "the built-in evicts only what is older than the window");
    v.push(102.0, 2_001);
    assert_eq!(v.sample_count(), 2);
}

#[test]
fn bad_prices_are_ignored() {
    let mut v = Volatility::new(60);
    for p in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        v.push(p, 1);
    }
    assert_eq!(v.sample_count(), 0);
}
