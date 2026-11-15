use std::collections::BTreeMap;

use bb_strategy_sdk::{OrderBook, Side};
use rust_decimal::Decimal;

use super::close_reference;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn book() -> OrderBook {
    OrderBook {
        bids: BTreeMap::from([(d("99"), d("1"))]),
        asks: BTreeMap::from([(d("101"), d("1"))]),
        last_update_id: 0,
    }
}

const STALE_MS: u64 = 10_000;

#[test]
fn a_stale_book_and_no_fresh_mark_give_no_reference() {
    let book = book();
    let reference =
        close_reference(Some(&book), Some(STALE_MS + 1), Side::Sell, None, STALE_MS, d("200"));
    assert_eq!(reference, None, "the caller's own fallback prices it, never the stale book");
    let never = close_reference(Some(&book), None, Side::Sell, None, STALE_MS, d("200"));
    assert_eq!(never, None, "a book of no known age is not fresh");
}

#[test]
fn a_fresh_book_with_no_mark_prices_off_its_mid_and_touch() {
    let book = book();
    let sell = close_reference(Some(&book), Some(0), Side::Sell, None, STALE_MS, d("200"));
    assert_eq!(sell, Some((d("100"), d("99"))), "mid, and the bid a sell crosses at");
    let buy = close_reference(Some(&book), Some(STALE_MS), Side::Buy, None, STALE_MS, d("200"));
    assert_eq!(buy, Some((d("100"), d("101"))), "fresh up to the stale window itself");
}

#[test]
fn a_fresh_book_far_from_a_fresh_mark_gives_the_mark() {
    let book = book();
    let far = close_reference(Some(&book), Some(0), Side::Sell, Some(d("90")), STALE_MS, d("200"));
    assert_eq!(far, Some((d("90"), d("90"))));
    let near =
        close_reference(Some(&book), Some(0), Side::Sell, Some(d("100.5")), STALE_MS, d("200"));
    assert_eq!(near, Some((d("100"), d("99"))), "the book agrees with the mark");
}
