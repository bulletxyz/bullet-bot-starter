//! The ladder's retry rules (`retry.rs`): what is placed again, when, and for how much.

use super::*;

/// The venue refuses a post-only order that would cross: a buy at or above 72 on a 70 / 72 book.
fn crossing_buys(o: &NewOrder) -> Option<&'static str> {
    (o.side == Side::Buy && o.price >= d("72")).then_some("a post-only order would cross the book")
}

fn states(sim: &SimHost<Ladder>) -> Vec<LevelState> {
    sim.strategy.report().levels.iter().map(|r| r.state).collect()
}

#[test]
fn a_level_refused_for_crossing_is_placed_again_once_the_price_no_longer_crosses() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let buy = placed(&sent)[0].clone();
    // The price falls through the buy: the level is placed again at its price and refused.
    sim.market.book = Some(bb_strategy_sdk::testing::book("70", "72", "10"));
    sim.refuse = crossing_buys;
    let again = sim.send(&fill_of(&buy));
    assert_eq!(placed(&again).len(), 1);
    assert_eq!(states(&sim), [LevelState::Refused, LevelState::Resting]);
    // Still crossing: nothing is sent, however often it is looked at.
    for _ in 0..3 {
        assert_eq!(sim.send(&Event::Book).len(), 0);
        assert_eq!(sim.send(&Event::Tick).len(), 0);
    }
    // The price comes back above the level: it is placed again, under a new client id.
    sim.market.book = Some(bb_strategy_sdk::testing::book("75", "76", "10"));
    sim.refuse = |_| None;
    let retried = sim.send(&Event::Book);
    let retried = placed(&retried);
    assert_eq!(retried.len(), 1, "{retried:?}");
    assert_eq!((retried[0].side, retried[0].price), (buy.side, buy.price));
    assert_ne!(retried[0].client_id, buy.client_id);
    assert_eq!(states(&sim), [LevelState::Resting, LevelState::Resting]);
    assert_eq!(sim.send(&Event::Book).len(), 0, "once it rests it is left alone");
}

#[test]
fn a_sell_refused_for_crossing_is_retried_above_the_best_bid() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let sell = placed(&sent)[1].clone();
    sim.market.book = Some(bb_strategy_sdk::testing::book("80", "81", "10"));
    sim.refuse = |o| {
        (o.side == Side::Sell && o.price <= d("80"))
            .then_some("a post-only order would cross the book")
    };
    sim.send(&fill_of(&sell));
    assert_eq!(states(&sim)[1], LevelState::Refused);
    sim.market.book = Some(bb_strategy_sdk::testing::book("75", "76", "10"));
    sim.refuse = |_| None;
    let retried = sim.send(&Event::Book);
    assert_eq!(placed(&retried).len(), 1);
    assert_eq!(states(&sim)[1], LevelState::Resting);
}

#[test]
fn a_level_refused_for_another_reason_stays_refused_even_when_the_book_moves() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    sim.refuse = |o| (o.side == Side::Buy).then_some("risk gate: max position");
    sim.start().unwrap();
    sim.market.book = Some(bb_strategy_sdk::testing::book("70", "72", "10"));
    sim.send(&Event::Book);
    sim.market.book = Some(bb_strategy_sdk::testing::book("75", "76", "10"));
    assert_eq!(sim.send(&Event::Book).len(), 0);
    assert_eq!(states(&sim)[0], LevelState::Refused);
}

fn cancelled(sell: &NewOrder) -> Event {
    Event::Order(bb_strategy_sdk::Order {
        id: "x".into(),
        client_id: sell.client_id.clone(),
        symbol: sell.symbol.clone(),
        side: sell.side,
        order_type: sell.order_type,
        price: sell.price,
        quantity: sell.quantity,
        filled_quantity: d("0"),
        status: OrderStatus::Cancelled,
    })
}

#[test]
fn levels_cancelled_by_someone_else_are_placed_again_on_the_next_good_book_exactly_once() {
    let mut sim = host(2, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let all: Vec<NewOrder> = placed(&sent).into_iter().cloned().collect();
    assert_eq!(all.len(), 4);
    // The venue (or the owner) sweeps everything that rests.
    for order in &all {
        assert_eq!(sim.send(&cancelled(order)).len(), 0);
    }
    assert!(states(&sim).iter().all(|s| *s == LevelState::Refused));
    // No usable book: nothing is placed.
    sim.market.book = Some(bb_strategy_sdk::OrderBook::default());
    assert_eq!(sim.send(&Event::Book).len(), 0);
    // A good book: each level goes out once, at its own price.
    sim.market.book = Some(bb_strategy_sdk::testing::book("75", "76", "10"));
    let again = sim.send(&Event::Book);
    let mut want: Vec<_> = all.iter().map(|o| (o.side, o.price)).collect();
    let mut got: Vec<_> = placed(&again).iter().map(|o| (o.side, o.price)).collect();
    want.sort_by_key(|x| (x.0 as u8, x.1));
    got.sort_by_key(|x| (x.0 as u8, x.1));
    assert_eq!(got, want);
    assert!(states(&sim).iter().all(|s| *s == LevelState::Resting));
    assert_eq!(sim.send(&Event::Book).len(), 0, "exactly once");
    assert_eq!(sim.send(&Event::Tick).len(), 0);
}

#[test]
fn a_cancelled_level_whose_price_crosses_waits_for_the_book_to_move_off_it() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let buy = placed(&sent)[0].clone();
    sim.market.book = Some(bb_strategy_sdk::testing::book("70", "72", "10"));
    sim.send(&cancelled(&buy));
    assert_eq!(sim.send(&Event::Book).len(), 0, "a buy at 75.4 would cross a 70 / 72 book");
    sim.market.book = Some(bb_strategy_sdk::testing::book("75", "76", "10"));
    assert_eq!(placed(&sim.send(&Event::Book)).len(), 1);
}

#[test]
fn the_refusal_reason_decides_whether_a_level_is_retried_when_the_venue_gives_one() {
    // A post-only cross is retried even when the ladder's own view of the book does not show it
    // (the venue's book was newer)...
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    sim.refuse = |o| (o.side == Side::Buy).then_some("a post-only order would cross the book");
    sim.start().unwrap();
    assert_eq!(states(&sim)[0], LevelState::Refused);
    sim.refuse = |_| None;
    assert_eq!(placed(&sim.send(&Event::Book)).len(), 1);
    assert_eq!(states(&sim)[0], LevelState::Resting);
    // ...and another reason is final even when the view's book crosses the level.
    let mut sim = host(1, Market::new("BTC-USD", "70", "72", NOON));
    sim.refuse = |o| (o.side == Side::Buy).then_some("risk gate: max position");
    sim.start().unwrap();
    sim.market.book = Some(bb_strategy_sdk::testing::book("75", "76", "10"));
    assert_eq!(sim.send(&Event::Book).len(), 0);
    assert_eq!(states(&sim)[0], LevelState::Refused);
}

fn cancel_all_of(sim: &mut SimHost<Ladder>, orders: &[NewOrder]) {
    for order in orders {
        sim.send(&cancelled(order));
    }
}

fn open(sim: &SimHost<Ladder>) -> Vec<NewOrder> {
    // The orders the ladder last asked for, as the venue holds them: the latest per client id.
    placed(&sim.sent).into_iter().cloned().collect()
}

#[test]
fn the_stop_phrase_matches_the_host() {
    // bb-strategy's STOPPED_REFUSAL, pinned here: the bot refuses with these words.
    assert_eq!(STOPPED_PHRASE, "new orders are stopped");
}

#[test]
fn a_level_refused_because_orders_are_stopped_keeps_its_retry_for_the_next_good_book() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    sim.refuse = |_| Some("risk gate: new orders are stopped: the book is stale (31 s old)");
    sim.start().unwrap();
    assert!(states(&sim).iter().all(|s| *s == LevelState::Refused));
    sim.refuse = |_| None;
    // A tick, even on a good book, retries nothing: only a book event does.
    assert_eq!(sim.send(&Event::Tick).len(), 0);
    assert_eq!(placed(&sim.send(&Event::Book)).len(), 2);
    assert!(states(&sim).iter().all(|s| *s == LevelState::Resting));
}

#[test]
fn only_the_venues_exact_post_only_phrase_is_a_cross() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    sim.refuse = |o| (o.side == Side::Buy).then_some("insufficient cross margin");
    sim.start().unwrap();
    sim.refuse = |_| None;
    assert_eq!(sim.send(&Event::Book).len(), 0, "cross margin is a gate limit, not a cross");
    assert_eq!(states(&sim)[0], LevelState::Refused);
}

#[test]
fn a_partly_filled_level_that_is_cancelled_is_placed_again_for_its_remainder() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let buy = placed(&sent)[0].clone();
    let part = NewOrder { quantity: d("0.004"), ..buy.clone() };
    assert_eq!(sim.send(&fill_of(&part)).len(), 0, "a part fill places nothing");
    sim.send(&cancelled(&buy));
    let again = sim.send(&Event::Book);
    let again = placed(&again);
    assert_eq!(again.len(), 1);
    assert_eq!((again[0].side, again[0].quantity), (Side::Buy, d("0.006")));
    // Its fill completes the level, which is then placed again whole.
    let rest = NewOrder { client_id: again[0].client_id.clone(), ..again[0].clone() };
    let whole = sim.send(&fill_of(&rest));
    assert_eq!(placed(&whole)[0].quantity, d("0.01"));
}

#[test]
fn a_level_the_venue_keeps_cancelling_is_held_after_three_and_says_so() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    sim.start().unwrap();
    for round in 1..=3 {
        sim.market.now_ms += 1_000;
        sim.send(&Event::Book);
        let buy = open(&sim).into_iter().rev().find(|o| o.side == Side::Buy).unwrap();
        cancel_all_of(&mut sim, &[buy]);
        assert_eq!(placed(&sim.send(&Event::Book)).len(), 1, "retry {round}");
    }
    sim.market.now_ms += 1_000;
    let buy = open(&sim).into_iter().rev().find(|o| o.side == Side::Buy).unwrap();
    cancel_all_of(&mut sim, &[buy]);
    assert_eq!(sim.send(&Event::Book).len(), 0, "held");
    assert_eq!(states(&sim)[0], LevelState::Held);
    assert!(sim.strategy.report().headline().contains("1 held"));
}

#[test]
fn gaps_and_quiet_books_do_not_count_toward_the_cap() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    sim.start().unwrap();
    for _ in 0..6 {
        // A depth gap: an empty book, then the sweep of everything resting.
        sim.market.now_ms += 1_000;
        sim.market.book = Some(bb_strategy_sdk::OrderBook::default());
        sim.send(&Event::Book);
        let resting: Vec<NewOrder> = open(&sim).into_iter().rev().take(2).collect();
        cancel_all_of(&mut sim, &resting);
        sim.market.book = Some(bb_strategy_sdk::testing::book("75", "76", "10"));
        assert_eq!(placed(&sim.send(&Event::Book)).len(), 2, "back after every gap");
    }
    assert!(states(&sim).iter().all(|s| *s == LevelState::Resting));
    // A sweep when no book has come for a while (a stale spell) is not the venue's doing either.
    for _ in 0..6 {
        sim.market.now_ms += 60_000;
        let resting: Vec<NewOrder> = open(&sim).into_iter().rev().take(2).collect();
        cancel_all_of(&mut sim, &resting);
        assert_eq!(placed(&sim.send(&Event::Book)).len(), 2);
    }
    assert!(states(&sim).iter().all(|s| *s == LevelState::Resting));
}

#[test]
fn a_sweep_six_seconds_after_the_last_book_does_not_count() {
    // A stale window of 5 s: the sweep's cancels come after a healed book, inside any fixed
    // quiet time. Everything cancelled at once is a sweep, not the venue refusing a level.
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    sim.start().unwrap();
    for _ in 0..4 {
        sim.send(&Event::Book);
        sim.market.now_ms += 6_000;
        let resting: Vec<NewOrder> = open(&sim).into_iter().rev().take(2).collect();
        cancel_all_of(&mut sim, &resting);
        assert_eq!(placed(&sim.send(&Event::Book)).len(), 2);
    }
    assert!(states(&sim).iter().all(|s| *s == LevelState::Resting), "{:?}", states(&sim));
}

#[test]
fn the_rate_phrase_matches_the_host() {
    // bb-strategy's RATE_REFUSAL, pinned here: the bot refuses with these words.
    assert_eq!(RATE_PHRASE, "the order rate limit is reached");
}

#[test]
fn a_level_refused_for_the_order_rate_is_retried_on_a_later_book_and_capped() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    sim.refuse = |_| Some("risk gate: the order rate limit is reached");
    sim.start().unwrap();
    assert!(states(&sim).iter().all(|s| *s == LevelState::Refused));
    sim.refuse = |_| None;
    assert_eq!(placed(&sim.send(&Event::Book)).len(), 2, "back on the next good book");
    // Refused again in each minute-long rate window within ten minutes: held after the cap, not
    // tried for ever.
    sim.refuse = |_| Some("risk gate: the order rate limit is reached");
    for i in 0..5 {
        sim.market.now_ms += 61_000;
        let resting: Vec<NewOrder> = open(&sim).into_iter().rev().take(2).collect();
        cancel_all_of(&mut sim, &resting);
        sim.send(&Event::Book);
        sim.send(&Event::Book);
        let _ = i;
    }
    assert!(states(&sim).contains(&LevelState::Held), "{:?}", states(&sim));
}

#[test]
fn a_late_partial_fill_on_the_cancelled_order_is_not_placed_twice() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let buy = placed(&sent)[0].clone();
    // The cancel arrives with 0.004 filled that no Trade told the ladder about yet.
    let mut event = cancelled(&buy);
    if let Event::Order(order) = &mut event {
        order.filled_quantity = d("0.004");
    }
    sim.send(&event);
    let again = sim.send(&Event::Book);
    let again = placed(&again);
    assert_eq!((again[0].side, again[0].quantity), (Side::Buy, d("0.006")));
}

#[test]
fn a_crossed_first_book_lays_nothing() {
    let mut market = Market::new("BTC-USD", "75", "76", NOON);
    market.book = Some(bb_strategy_sdk::testing::book("76", "75", "10"));
    let mut sim = host(1, market);
    assert_eq!(sim.start().unwrap().len(), 0);
    assert_eq!(sim.send(&Event::Book).len(), 0);
    sim.market.book = Some(bb_strategy_sdk::testing::book("75", "76", "10"));
    assert_eq!(placed(&sim.send(&Event::Book)).len(), 2, "laid on the first good book");
}

fn cancelled_with(order: &NewOrder, filled: &str) -> Event {
    let mut event = cancelled(order);
    if let Event::Order(o) = &mut event {
        o.filled_quantity = d(filled);
    }
    event
}

#[test]
fn a_trade_after_the_cancel_that_reported_it_is_not_counted_twice() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let buy = placed(&sent)[0].clone();
    sim.send(&cancelled_with(&buy, "0.006"));
    // The same 0.006 arrives as a trade before the next book.
    let part = NewOrder { quantity: d("0.006"), ..buy.clone() };
    assert_eq!(sim.send(&fill_of(&part)).len(), 0);
    let again = sim.send(&Event::Book);
    let again = placed(&again);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].quantity, d("0.004"), "0.01 - 0.006, not a whole order");
}

#[test]
fn a_remainder_order_cancelled_with_a_fill_no_trade_reported_is_placed_for_what_is_left() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let buy = placed(&sent)[0].clone();
    sim.send(&cancelled_with(&buy, "0.004"));
    let rest = placed(&sim.send(&Event::Book))[0].clone();
    assert_eq!(rest.quantity, d("0.006"));
    // The remainder is cancelled having filled 0.003 that no trade told the ladder.
    sim.send(&cancelled_with(&rest, "0.003"));
    let again = sim.send(&Event::Book);
    let again = placed(&again);
    assert_eq!(again[0].quantity, d("0.003"), "0.01 - 0.004 - 0.003");
}

#[test]
fn a_rate_refusal_counts_toward_the_cap_once_a_minute() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    sim.refuse = |_| Some("risk gate: the order rate limit is reached");
    sim.start().unwrap();
    // The same rate window: every book update retries and is refused again, counted once.
    for _ in 0..20 {
        sim.market.now_ms += 1_000;
        sim.send(&Event::Book);
    }
    assert!(!states(&sim).contains(&LevelState::Held), "{:?}", states(&sim));
    // Windows a minute apart each count once: held after the cap.
    for _ in 0..5 {
        sim.market.now_ms += 61_000;
        sim.send(&Event::Book);
    }
    assert!(states(&sim).contains(&LevelState::Held), "{:?}", states(&sim));
}

#[path = "ledger_tests.rs"]
mod ledger;
