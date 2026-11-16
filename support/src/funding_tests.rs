#![expect(clippy::arithmetic_side_effects, reason = "test arithmetic")]

use super::*;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}
fn series(now_ms: i64, hours: i64, rate: &str) -> Vec<FundingSample> {
    (0..hours).map(|i| FundingSample { time_ms: now_ms - i * HOUR_MS, rate_8h: d(rate) }).collect()
}

#[test]
fn book_walk_slippage_bps() {
    let asks = [(d("100"), d("1")), (d("101"), d("1"))];
    // VWAP 100.333.. vs mid 99.5.
    let slip = walk_slip_bps(&asks, true, d("99.5"), d("1.5")).unwrap();
    assert_eq!(slip.round_dp(2), d("83.75"));
    let bids = [(d("99"), d("1")), (d("98"), d("1"))];
    let slip = walk_slip_bps(&bids, false, d("99.5"), d("1.5")).unwrap();
    assert_eq!(slip.round_dp(2), d("83.75"));
    assert_eq!(walk_slip_bps(&[], true, d("99.5"), d("1")), None, "empty side");
    assert_eq!(walk_slip_bps(&asks, true, d("99.5"), d("2.5")), None, "too thin");
    // Crossed: an ask below mid is no cost, not a rebate.
    assert_eq!(walk_slip_bps(&asks, true, d("101"), d("1")), Some(Decimal::ZERO));
}

#[test]
fn overflow_is_none_not_a_panic() {
    let huge = [(Decimal::MAX, d("2"))];
    assert_eq!(walk_slip_bps(&huge, true, d("1"), d("2")), None, "px * take overflows");
    // Each level fits, but the running cost across two levels does not.
    let half = Decimal::MAX / Decimal::TWO + Decimal::ONE;
    let two_levels = [(half, d("1")), (half, d("1"))];
    assert_eq!(walk_slip_bps(&two_levels, true, d("1"), d("2")), None, "cost sum overflows");
    // The walk fits, but `past / mid * 10_000` does not.
    let far = [(Decimal::MAX / d("2"), d("1"))];
    assert_eq!(walk_slip_bps(&far, true, d("1"), d("1")), None, "bps scaling overflows");
}

#[test]
fn trailing_median_ignores_one_spike_the_mean_chases() {
    let now = 1_000 * HOUR_MS;
    let mut a = series(now, 24, "0.0001");
    a[0].rate_8h = d("0.03");
    assert_eq!(trailing_median(&a, now, 24), Some(d("0.0001")));
}

#[test]
fn trailing_median_of_an_even_count_averages_the_middle_pair() {
    let now = 1_000 * HOUR_MS;
    let mut a = series(now, 4, "0.0001");
    a[0].rate_8h = d("0.0005");
    a[1].rate_8h = d("0.0003");
    assert_eq!(trailing_median(&a, now, 4), Some(d("0.0002")));
}

#[test]
fn trailing_median_keeps_the_coverage_rule_and_sign() {
    let now = 1_000 * HOUR_MS;
    assert_eq!(trailing_median(&series(now, 7, "0.0001"), now, 10), None);
    assert_eq!(trailing_median(&[], now, 0), None, "no divide by zero");
    assert_eq!(trailing_median(&series(now, 5, "-0.0002"), now, 5), Some(d("-0.0002")));
}

#[test]
fn trailing_median_overflow_is_none() {
    let now = 1_000 * HOUR_MS;
    let mut a = series(now, 4, "0");
    for s in &mut a {
        s.rate_8h = Decimal::MAX / Decimal::TWO + Decimal::ONE;
    }
    assert_eq!(trailing_median(&a, now, 4), None, "the middle pair's sum overflows");
}

#[test]
fn trailing_median_ignores_samples_outside_the_window_and_duplicates() {
    let now = 1_000 * HOUR_MS;
    let at = |time_ms: i64, rate: &str| FundingSample { time_ms, rate_8h: d(rate) };
    // Three hours of a window of four: the median is the middle one, 0.0002.
    let mut a =
        vec![at(now, "0.0001"), at(now - HOUR_MS, "0.0002"), at(now - 2 * HOUR_MS, "0.0003")];
    // Any one of these counted would move it.
    a.push(at(now - 4 * HOUR_MS, "0.9")); // just before the window
    a.push(at(now + HOUR_MS, "0.9")); // after `now`
    a.push(at(now, "0.9")); // the newest hour again: the first seen wins
    assert_eq!(trailing_median(&a, now, 4), Some(d("0.0002")));
}
