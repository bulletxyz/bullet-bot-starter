use std::collections::BTreeMap;

use super::*;

#[test]
fn clip_keeps_short_text_and_cuts_long_text_to_max_characters() {
    assert_eq!(clip("short", 10), "short");
    let long = "é".repeat(300);
    let cut = clip(&long, MAX_REPORT_TEXT);
    assert_eq!(cut.chars().count(), MAX_REPORT_TEXT);
    assert!(cut.ends_with('…'));
}

#[test]
fn would_cross_at_or_past_the_touch_and_not_on_an_empty_side() {
    let book = OrderBook {
        bids: BTreeMap::from([(Decimal::from(100), Decimal::ONE)]),
        asks: BTreeMap::from([(Decimal::from(101), Decimal::ONE)]),
        last_update_id: 0,
    };
    assert!(book.would_cross(Side::Buy, Decimal::from(101)));
    assert!(!book.would_cross(Side::Buy, Decimal::from(100)));
    assert!(book.would_cross(Side::Sell, Decimal::from(100)));
    assert!(!book.would_cross(Side::Sell, Decimal::from(101)));
    assert!(!OrderBook::default().would_cross(Side::Buy, Decimal::ONE));
}
