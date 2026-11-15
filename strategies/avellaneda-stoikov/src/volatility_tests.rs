use super::*;

const T0: u64 = 1_000_000;

#[test]
fn insufficient_samples() {
    let mut v = Volatility::new(60);
    assert_eq!(v.sigma(), None);
    v.push(100.0, T0);
    assert_eq!(v.sigma(), None);
    // Two samples gives one return — not enough for unbiased sample variance.
    v.push(101.0, T0 + 1);
    assert_eq!(v.sigma(), None);
    // Three samples gives two returns — now we have a valid estimate.
    v.push(102.0, T0 + 2);
    assert!(v.sigma().is_some());
}

#[test]
fn flat_price_zero_vol() {
    let mut v = Volatility::new(60);
    for i in 0..20 {
        v.push(100.0, T0 + i * 100);
    }
    let s = v.sigma().unwrap();
    assert!(s < 1e-9, "flat price should have ~0 vol, got {s}");
}

#[test]
fn trending_price_positive_vol() {
    let mut v = Volatility::new(60);
    for i in 0..20 {
        let mid = 100.0 + (i as f64) * 0.1;
        v.push(mid, T0 + i * 100);
    }
    let s = v.sigma().unwrap();
    // Log returns on a linear ramp decrease slightly but have small
    // variance; just assert non-zero and finite.
    assert!(s > 0.0 && s.is_finite(), "expected positive finite vol, got {s}");
}

#[test]
fn evicts_old_samples() {
    let mut v = Volatility::new(1); // 1-second window
    for i in 0..5 {
        v.push(100.0 + i as f64, T0 + i * 100);
    }
    assert_eq!(v.sample_count(), 5);
    // Jump well past the window — only the most recent sample should remain.
    v.push(200.0, T0 + 10_000);
    assert!(v.sample_count() <= 2, "expected eviction, got {}", v.sample_count());
}
