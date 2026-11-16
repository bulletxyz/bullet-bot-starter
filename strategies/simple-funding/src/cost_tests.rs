use std::collections::BTreeMap;

use super::*;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn book(bid: &str, ask: &str, qty: &str) -> OrderBook {
    OrderBook {
        bids: BTreeMap::from([(d(bid), d(qty))]),
        asks: BTreeMap::from([(d(ask), d(qty))]),
        last_update_id: 0,
    }
}

#[test]
fn round_trip_is_two_fees_plus_both_walks() {
    // 4.5bp fee twice, 1bp in, 2bp out: 12bp.
    let c = round_trip_cost(Some(d("4.5")), Some(d("1")), Some(d("2")));
    assert_eq!(c, Some(d("0.0012")));
    assert_eq!(round_trip_cost(None, Some(d("1")), Some(d("1"))), None);
    assert_eq!(round_trip_cost(Some(d("1")), None, Some(d("1"))), None);
    assert_eq!(round_trip_cost(Some(d("1")), Some(d("1")), None), None);
    assert_eq!(round_trip_cost(Some(Decimal::MAX), Some(d("1")), Some(d("1"))), None);
}

#[test]
fn assess_costs_the_half_spread_both_ways() {
    // mid 100, 20bp wide: 10bp past mid each way, plus 2 x 4.5bp.
    let c = assess(&book("99.9", "100.1", "1000"), Some(d("4.5")), d("1000"));
    assert_eq!(c.round_trip.map(|r| r.round_dp(6)), Some(d("0.0029")));
    assert_eq!(c.depth_usd, Some(d("99900")));
}

#[test]
fn a_book_too_thin_for_the_size_has_no_cost_but_has_depth() {
    let c = assess(&book("99.9", "100.1", "1"), Some(d("4.5")), d("1000"));
    assert_eq!(c.round_trip, None);
    assert_eq!(c.depth_usd, Some(d("99.9")));
}

#[test]
fn an_empty_side_or_unknown_fee_is_unknown() {
    let mut b = book("99.9", "100.1", "1000");
    assert_eq!(assess(&b, None, d("1000")).round_trip, None, "fee unknown");
    b.asks.clear();
    let c = assess(&b, Some(d("4.5")), d("1000"));
    assert_eq!((c.round_trip, c.depth_usd), (None, None));
    assert_eq!(assess(&OrderBook::default(), Some(d("4.5")), d("1000")).depth_usd, None);
}

#[test]
fn a_walk_beyond_the_cap_is_not_fillable() {
    let mut b = book("99.95", "100.05", "0.1");
    b.asks.insert(d("105"), d("1000"));
    b.bids.insert(d("95"), d("1000"));
    assert!(!fillable_within(&b, d("100"), d("10")), "depth lies 5% away, cap is 10 bp");
    assert!(fillable_within(&b, d("100"), d("600")), "a wide cap reaches it");
    assert!(fillable_within(&b, d("5"), d("10")), "a small size fits at the touch");
}

#[test]
fn a_thin_or_one_sided_book_is_not_fillable() {
    assert!(!fillable_within(&book("99.95", "100.05", "0.5"), d("100"), d("100")));
    let mut b = book("99.95", "100.05", "1000");
    b.bids.clear();
    assert!(!fillable_within(&b, d("100"), d("100")));
    assert!(fills_within(&book("99.95", "100.05", "1000"), true, d("1"), d("100.05")));
}

#[test]
fn zero_quantity_levels_are_not_depth() {
    let mut b = book("99.9", "100.1", "1000");
    b.asks.insert(d("100.2"), Decimal::ZERO);
    assert_eq!(levels(&b, true).len(), 1);
}
