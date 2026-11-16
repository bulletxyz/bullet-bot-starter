use super::*;

#[test]
fn signed_position_treats_absent_as_flat_and_sell_negative() {
    let pos = |symbol: &str, side, size: i64| Position {
        symbol: symbol.into(),
        side,
        size: Decimal::from(size),
        ..Position::default()
    };
    let ps = [pos("BTC", Some(Side::Sell), 2), pos("ETH", Some(Side::Buy), 3), pos("SOL", None, 1)];
    assert_eq!(signed_position(&ps, "BTC"), Decimal::from(-2));
    assert_eq!(signed_position(&ps, "ETH"), Decimal::from(3));
    assert_eq!(signed_position(&ps, "SOL"), Decimal::ZERO);
    assert_eq!(signed_position(&ps, "DOGE"), Decimal::ZERO, "absent = flat");
}

#[test]
fn orderbook_midpoint() {
    let mut book = OrderBook::default();
    book.bids.insert(Decimal::from(100), Decimal::from(1));
    book.asks.insert(Decimal::from(102), Decimal::from(1));
    assert_eq!(book.midpoint(), Some(Decimal::from(101)));
}

#[test]
fn orderbook_spread() {
    let mut book = OrderBook::default();
    book.bids.insert(Decimal::from(100), Decimal::from(1));
    book.asks.insert(Decimal::from(102), Decimal::from(1));
    assert_eq!(book.spread(), Some(Decimal::from(2)));
}

#[test]
fn orderbook_midpoint_and_spread_overflow_is_none() {
    let mut book = OrderBook::default();
    book.bids.insert(Decimal::MIN, Decimal::ONE);
    book.asks.insert(Decimal::MAX, Decimal::ONE);
    assert_eq!(book.spread(), None, "MAX - MIN overflows");
    let mut book = OrderBook::default();
    book.bids.insert(Decimal::MAX, Decimal::ONE);
    book.asks.insert(Decimal::MAX, Decimal::ONE);
    assert_eq!(book.midpoint(), None, "MAX + MAX overflows");
}

#[test]
fn orderbook_best_bid_ask() {
    let mut book = OrderBook::default();
    book.bids.insert(Decimal::from(99), Decimal::from(5));
    book.bids.insert(Decimal::from(100), Decimal::from(3));
    book.asks.insert(Decimal::from(101), Decimal::from(2));
    book.asks.insert(Decimal::from(102), Decimal::from(4));

    let best_bid = book.best_bid().unwrap();
    assert_eq!(best_bid.price, Decimal::from(100));
    assert_eq!(best_bid.quantity, Decimal::from(3));

    let best_ask = book.best_ask().unwrap();
    assert_eq!(best_ask.price, Decimal::from(101));
    assert_eq!(best_ask.quantity, Decimal::from(2));
}

#[test]
fn empty_orderbook() {
    let book = OrderBook::default();
    assert!(book.best_bid().is_none());
    assert!(book.best_ask().is_none());
    assert!(book.midpoint().is_none());
    assert!(book.spread().is_none());
}

#[test]
fn side_opposite() {
    assert_eq!(Side::Buy.opposite(), Side::Sell);
    assert_eq!(Side::Sell.opposite(), Side::Buy);
}

#[test]
fn would_cross_buy_at_or_above_best_ask() {
    let mut book = OrderBook::default();
    book.bids.insert(Decimal::from(100), Decimal::from(1));
    book.asks.insert(Decimal::from(101), Decimal::from(1));
    assert!(book.would_cross(Side::Buy, Decimal::from(101)));
    assert!(book.would_cross(Side::Buy, Decimal::from(102)));
    assert!(!book.would_cross(Side::Buy, Decimal::from(100)));
}

#[test]
fn would_cross_sell_at_or_below_best_bid() {
    let mut book = OrderBook::default();
    book.bids.insert(Decimal::from(100), Decimal::from(1));
    book.asks.insert(Decimal::from(101), Decimal::from(1));
    assert!(book.would_cross(Side::Sell, Decimal::from(100)));
    assert!(book.would_cross(Side::Sell, Decimal::from(99)));
    assert!(!book.would_cross(Side::Sell, Decimal::from(101)));
}

#[test]
fn would_cross_empty_side_returns_false() {
    // Sparse books happen during reconnect / at startup — don't block
    // placement just because the relevant side isn't seeded yet.
    let mut only_bids = OrderBook::default();
    only_bids.bids.insert(Decimal::from(100), Decimal::from(1));
    assert!(!only_bids.would_cross(Side::Buy, Decimal::from(1_000_000)));

    let mut only_asks = OrderBook::default();
    only_asks.asks.insert(Decimal::from(100), Decimal::from(1));
    assert!(!only_asks.would_cross(Side::Sell, Decimal::from(1)));
}
