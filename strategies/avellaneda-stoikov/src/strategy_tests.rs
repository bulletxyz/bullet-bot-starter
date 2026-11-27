//! A place whose outcome is unknown (a timeout, a 5xx: the order may be resting) is held, not
//! placed again; a later venue read adopts it or, after the settle margin, frees it.

use std::collections::BTreeMap;

use bb_strategy_sdk::testing::{Market, on_bullet};
use bb_strategy_sdk::{
    BULLET, Event, ExchangeIntent, Intent, MarketEvent, NewOrder, Order, OrderBook, OrderStatus,
    OrderType, Side, StrategyAdapter,
};
use rust_decimal::Decimal;
use serde_json::json;

use crate::config::AvellanedaStoikovConfig;
use crate::strategy::AvellanedaStoikov;

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

/// Two levels a side, no throttle, the periodic reconcile off.
#[expect(clippy::needless_pass_by_value, reason = "called with temporaries")]
fn config(extra: serde_json::Value) -> AvellanedaStoikovConfig {
    let mut c = json!({
        "symbol": "BTC-USD", "order_size": "0.01", "max_position": "0.03", "gamma": "0.1",
        "kappa": "1.5", "order_levels": 2, "min_refresh_interval_ms": 0,
        "reconcile_interval_secs": 0, "order_refresh_secs": 1,
    });
    for (k, v) in extra.as_object().unwrap() {
        c[k] = v.clone();
    }
    serde_json::from_value(c).unwrap()
}

const T: u64 = 1_794_398_400_000;

/// BTC-USD on Bullet with `book`, flat; `open` the venue's last read, begun at `read_started_ms`.
fn view(
    book: &OrderBook,
    open: Option<&[Order]>,
    now_ms: u64,
    read_started_ms: Option<u64>,
) -> Market {
    let mut m = Market::new("BTC-USD", "1", "2", now_ms);
    m.set_book(Some(book.clone()));
    let bullet = m.bullet_mut().unwrap();
    bullet.open_orders = open.map(<[Order]>::to_vec);
    bullet.positions = open.map(|_| Vec::new());
    bullet.read_started_ms = read_started_ms;
    m
}

fn book_event() -> Event {
    on_bullet("BTC-USD", MarketEvent::Book)
}

fn place(o: &NewOrder) -> Intent {
    Intent::on(BULLET, ExchangeIntent::Place(o.clone()))
}

fn places(intents: &[Intent]) -> Vec<NewOrder> {
    intents
        .iter()
        .filter_map(|i| {
            if let Intent::On { intent: ExchangeIntent::Place(o), .. } = i {
                Some(o.clone())
            } else {
                None
            }
        })
        .collect()
}

/// A started strategy and the four orders its ladder went out with.
fn quoted() -> (AvellanedaStoikov, Vec<NewOrder>) {
    let mut s = AvellanedaStoikov::new(config(json!({})));
    s.start(&view(&book("75000", "75010"), None, T, None).view()).unwrap();
    assert!(
        s.on_event(&book_event(), &view(&book("75001", "75011"), None, T + 10, None).view())
            .is_empty()
    );
    let out = s.on_event(&book_event(), &view(&book("75002", "75012"), None, T + 20, None).view());
    let orders = places(&out);
    assert_eq!(orders.len(), 4, "{out:?}");
    (s, orders)
}

fn unanswered(s: &mut AvellanedaStoikov, orders: &[NewOrder], now_ms: u64) {
    for o in orders {
        let event = Event::Unknown { intent: place(o), error: "timeout".into() };
        assert!(
            s.on_event(&event, &view(&book("75002", "75012"), None, now_ms, None).view())
                .is_empty()
        );
    }
}

fn resting(o: &NewOrder, id: &str) -> Order {
    Order {
        id: id.into(),
        client_id: o.client_id.clone(),
        symbol: "BTC-USD".into(),
        side: o.side,
        order_type: OrderType::PostOnly,
        price: o.price,
        quantity: o.quantity,
        filled_quantity: Decimal::ZERO,
        status: OrderStatus::Open,
    }
}

fn slots_with_ids(s: &AvellanedaStoikov) -> Vec<bool> {
    s.report().slots.iter().map(|q| q.order_id.is_some()).collect()
}

#[test]
fn an_unanswered_place_is_held_and_not_placed_again() {
    let (mut s, orders) = quoted();
    unanswered(&mut s, &orders, T + 30);
    assert_eq!(s.report().slots.len(), 4, "the four levels are held");
    // The next book finds every level taken: nothing to place.
    let again =
        s.on_event(&book_event(), &view(&book("75003", "75013"), None, T + 40, None).view());
    assert!(places(&again).is_empty(), "placed again: {again:?}");
}

#[test]
fn a_place_the_host_failed_to_send_is_placed_again() {
    let (mut s, orders) = quoted();
    for o in &orders {
        let event = Event::Failed { intent: place(o), error: "not sent".into() };
        s.on_event(&event, &view(&book("75002", "75012"), None, T + 30, None).view());
    }
    assert!(s.report().slots.is_empty());
    let again =
        s.on_event(&book_event(), &view(&book("75003", "75013"), None, T + 40, None).view());
    assert_eq!(places(&again).len(), 4, "{again:?}");
}

#[test]
fn a_read_that_lists_a_held_order_adopts_it() {
    let (mut s, orders) = quoted();
    unanswered(&mut s, &orders, T + 30);
    let listed: Vec<Order> =
        orders.iter().enumerate().map(|(i, o)| resting(o, &format!("oid-{i}"))).collect();
    let b = book("75002", "75012");
    s.on_event(&Event::Tick, &view(&b, Some(&listed), T + 500, Some(T + 500)).view());
    assert_eq!(slots_with_ids(&s), [true; 4], "each took its venue id");
    let again =
        s.on_event(&book_event(), &view(&book("75003", "75013"), None, T + 600, None).view());
    assert!(places(&again).is_empty(), "{again:?}");
}

#[test]
fn a_read_after_the_settle_margin_that_omits_it_frees_the_level() {
    let (mut s, orders) = quoted();
    unanswered(&mut s, &orders, T + 30);
    // The venue lists three of the four, in a read that began more than the margin later.
    let listed: Vec<Order> =
        orders[1..].iter().enumerate().map(|(i, o)| resting(o, &format!("oid-{i}"))).collect();
    let b = book("75002", "75012");
    // The tick that frees it requotes the level at once.
    let out = s.on_event(&Event::Tick, &view(&b, Some(&listed), T + 2_600, Some(T + 2_500)).view());
    let again = places(&out);
    assert_eq!(again.len(), 1, "only the freed level is placed again: {out:?}");
    assert_eq!((again[0].side, again[0].price.round_dp(0)), (Side::Buy, d("74969")));
    assert_eq!(s.report().slots.len(), 3, "the freed one has no slot until it is answered");
}

#[test]
fn a_read_just_after_the_unknown_place_does_not_free_it() {
    let (mut s, orders) = quoted();
    unanswered(&mut s, &orders, T + 30);
    // The venue's list can lag: a read that began within the margin omitting the order proves
    // nothing.
    let b = book("75002", "75012");
    s.on_event(&Event::Tick, &view(&b, Some(&[]), T + 600, Some(T + 500)).view());
    assert_eq!(s.report().slots.len(), 4);
    let again =
        s.on_event(&book_event(), &view(&book("75003", "75013"), None, T + 700, None).view());
    assert!(places(&again).is_empty(), "{again:?}");
}

#[test]
fn a_held_order_is_read_back_on_the_next_tick() {
    let (mut s, orders) = quoted();
    assert!(!s.wants_venue(T + 100, false), "nothing is due before an order is unknown");
    unanswered(&mut s, &orders[..1], T + 30);
    assert!(s.wants_venue(T + 100, false), "a held order needs a read to settle");
}

#[test]
fn an_unanswered_amend_holds_both_the_old_quote_and_the_new_one() {
    let (mut s, orders) = quoted();
    let b = book("75002", "75012");
    for (i, o) in orders.iter().enumerate() {
        let result = bb_strategy_sdk::OrderResult {
            order_id: Some(format!("oid-{i}")),
            client_id: o.client_id.clone(),
            success: true,
            error: None,
        };
        s.on_event(
            &Event::Outcome { intent: place(o), result },
            &view(&b, None, T + 30, None).view(),
        );
    }
    // The market moves: every rung is amended, and the first amend's call times out.
    let moved = book("75100", "75110");
    let out = s.on_event(&book_event(), &view(&moved, None, T + 100, None).view());
    let amends: Vec<_> = out
        .iter()
        .filter(|i| matches!(i, Intent::On { intent: ExchangeIntent::Amend(_), .. }))
        .cloned()
        .collect();
    assert_eq!(amends.len(), 4, "{out:?}");
    let event = Event::Unknown { intent: amends[0].clone(), error: "timeout".into() };
    s.on_event(&event, &view(&moved, None, T + 110, None).view());
    assert_eq!(
        s.report().slots.len(),
        1 + 1,
        "the old quote stays tracked and the new one is held"
    );
}
