use std::collections::BTreeMap;

use super::*;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn book(bid: &str, ask: &str) -> OrderBook {
    OrderBook {
        bids: BTreeMap::from([(d(bid), d("1"))]),
        asks: BTreeMap::from([(d(ask), d("1"))]),
        last_update_id: 0,
    }
}

#[test]
fn mid_needs_two_uncrossed_sides() {
    assert_eq!(two_sided_mid(&book("99", "101")), Some(d("100")));
    assert_eq!(two_sided_mid(&book("101", "99")), None, "crossed");
    assert_eq!(two_sided_mid(&book("100", "100")), None, "locked");
    assert_eq!(two_sided_mid(&OrderBook::default()), None, "empty");
}

#[test]
fn within_bps_is_inclusive() {
    assert!(within_bps(d("102"), d("100"), d("200")));
    assert!(!within_bps(d("102.01"), d("100"), d("200")));
    assert!(within_bps(d("98"), d("100"), d("200")));
}

#[test]
fn the_cross_is_the_slippage_past_the_touch() {
    let buy = capped_ioc_price(Side::Buy, d("100.05"), d("100"), d("10"), d("200"));
    assert_eq!(buy, Some(d("100.05") * d("1.001")));
    let sell = capped_ioc_price(Side::Sell, d("99.95"), d("100"), d("10"), d("200"));
    assert_eq!(sell, Some(d("99.95") * d("0.999")));
}

#[test]
fn a_wide_cross_stops_at_the_cap_around_the_reference() {
    // 10% of slippage, capped at 2% from the mid.
    assert_eq!(
        capped_ioc_price(Side::Buy, d("100.05"), d("100"), d("1000"), d("200")),
        Some(d("102"))
    );
    assert_eq!(
        capped_ioc_price(Side::Sell, d("99.95"), d("100"), d("1000"), d("200")),
        Some(d("98"))
    );
}

#[test]
fn a_non_positive_price_is_none() {
    assert_eq!(capped_ioc_price(Side::Sell, d("1"), d("1"), d("20000"), d("20000")), None);
}
