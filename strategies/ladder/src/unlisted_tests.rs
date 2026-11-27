//! A level whose placement has no answer, or an unknown one (it may rest): held, and settled by a
//! fresh venue read that began at least [`UNLISTED_AFTER_MS`] after the order was sent. One the
//! read lists rests; one it omits never landed and is placed again on a later book. Nothing is
//! settled without a fresh read or its start.

use bb_strategy_sdk::testing::{Market, d, placed};
use bb_strategy_sdk::{Event, Intent, NewOrder, Order, OrderStatus, OrderType, StrategyAdapter};

use super::*;

const NOON: u64 = 1_794_398_400_000;

fn ladder() -> (Ladder, Market, NewOrder) {
    let settings = LadderSettings {
        levels: 1,
        spacing_bps: d("10"),
        order_size: d("0.01"),
        order_type: OrderType::PostOnly,
    };
    let mut ladder = Ladder::new(settings).unwrap();
    let market = Market::new("BTC-USD", "75", "76", NOON);
    let sent = ladder.start(&market.view()).unwrap();
    let (buy, sell) = (placed(&sent)[0].clone(), placed(&sent)[1].clone());
    // The sell rests: only the buy's placement is left without an answer.
    let ok = outcome(&place(sell), Ok("v2"));
    ladder.on_event(&ok, &market.view());
    (ladder, market, buy)
}

fn unknown(order: &NewOrder) -> Event {
    Event::Unknown { intent: place(order.clone()), error: "timed out".into() }
}

fn resting(order: &NewOrder) -> Order {
    Order {
        id: "v1".into(),
        client_id: order.client_id.clone(),
        symbol: order.symbol.clone(),
        side: order.side,
        order_type: order.order_type,
        price: order.price,
        quantity: order.quantity,
        filled_quantity: d("0"),
        status: OrderStatus::Open,
    }
}

/// A tick at `now_ms` whose venue read began at `started` and lists `open`.
fn read(ladder: &mut Ladder, market: &mut Market, open: Vec<Order>, started: Option<u64>) {
    market.now_ms = started.map_or(market.now_ms, |s| s.saturating_add(5));
    let bullet = market.bullet_mut().unwrap();
    bullet.open_orders = Some(open);
    bullet.read_started_ms = started;
    assert_eq!(ladder.on_event(&Event::Tick, &market.view()).len(), 0, "a tick places nothing");
    let bullet = market.bullet_mut().unwrap();
    bullet.open_orders = None;
    bullet.read_started_ms = None;
}

fn book(ladder: &mut Ladder, market: &mut Market) -> Vec<NewOrder> {
    market.now_ms = market.now_ms.saturating_add(10);
    placed(&ladder.on_event(&book_event(), &market.view())).into_iter().cloned().collect()
}

fn state(ladder: &Ladder) -> LevelState {
    ladder.report().levels[0].state
}

#[test]
fn an_unknown_placement_a_late_read_omits_is_placed_again_on_the_next_book() {
    let (mut ladder, mut market, buy) = ladder();
    ladder.on_event(&unknown(&buy), &market.view());
    assert_eq!(state(&ladder), LevelState::Pending, "held while it may rest");
    assert_eq!(book(&mut ladder, &mut market).len(), 0, "never placed twice while held");
    read(&mut ladder, &mut market, Vec::new(), Some(NOON + UNLISTED_AFTER_MS));
    let again = book(&mut ladder, &mut market);
    assert_eq!(again.len(), 1, "{again:?}");
    assert_eq!((again[0].side, again[0].price), (buy.side, buy.price));
    assert_ne!(again[0].client_id, buy.client_id, "under a new client id");
}

#[test]
fn a_placement_with_no_answer_a_late_read_omits_is_placed_again_too() {
    let (mut ladder, mut market, buy) = ladder();
    read(&mut ladder, &mut market, Vec::new(), Some(NOON + UNLISTED_AFTER_MS + 1));
    assert_eq!(book(&mut ladder, &mut market).len(), 1);
    let _ = buy;
}

#[test]
fn a_read_that_lists_it_rests_it_and_it_is_never_placed_twice() {
    let (mut ladder, mut market, buy) = ladder();
    ladder.on_event(&unknown(&buy), &market.view());
    read(&mut ladder, &mut market, vec![resting(&buy)], Some(NOON + UNLISTED_AFTER_MS));
    assert_eq!(state(&ladder), LevelState::Resting);
    assert_eq!(book(&mut ladder, &mut market).len(), 0);
    read(&mut ladder, &mut market, Vec::new(), Some(NOON + 3 * UNLISTED_AFTER_MS));
    assert_eq!(book(&mut ladder, &mut market).len(), 0, "a resting order is left to its events");
}

#[test]
fn a_read_too_soon_after_the_send_or_without_its_start_settles_nothing() {
    let (mut ladder, mut market, buy) = ladder();
    ladder.on_event(&unknown(&buy), &market.view());
    // Began less than the wait after the send, though it reaches the ladder long after.
    read(&mut ladder, &mut market, Vec::new(), Some(NOON + UNLISTED_AFTER_MS - 1));
    market.now_ms = NOON + 10 * UNLISTED_AFTER_MS;
    assert_eq!(book(&mut ladder, &mut market).len(), 0);
    // A fresh list with no start: when it began is unknown, so nothing is freed.
    market.now_ms = NOON + 20 * UNLISTED_AFTER_MS;
    read(&mut ladder, &mut market, Vec::new(), None);
    assert_eq!(book(&mut ladder, &mut market).len(), 0);
    assert_eq!(state(&ladder), LevelState::Pending);
}

#[test]
fn a_fill_of_the_freed_order_still_counts_and_overfills_nothing() {
    let (mut ladder, mut market, buy) = ladder();
    ladder.on_event(&unknown(&buy), &market.view());
    read(&mut ladder, &mut market, Vec::new(), Some(NOON + UNLISTED_AFTER_MS));
    let again = book(&mut ladder, &mut market);
    // The first one had filled after all; its trade arrives late.
    let late = fill_of(&buy);
    let out = ladder.on_event(&late, &market.view());
    assert_eq!(ladder.report().fills, 1);
    let cancels: Vec<_> = out
        .iter()
        .filter(|i| matches!(i, Intent::On { intent: ExchangeIntent::Cancel(_), .. }))
        .collect();
    assert_eq!(cancels.len(), 1, "the new order would overfill the level: {out:?}");
    assert!(
        matches!(cancels[0], Intent::On { intent: ExchangeIntent::Cancel(c), .. } if c.client_id == again[0].client_id)
    );
}
