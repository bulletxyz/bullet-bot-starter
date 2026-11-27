//! The level ledger through the whole ladder: late trades, cancels the ladder asks for,
//! completions.

use super::*;

fn cancels(intents: &[Intent]) -> Vec<&CancelOrder> {
    intents
        .iter()
        .filter_map(|i| match i {
            Intent::On { intent: ExchangeIntent::Cancel(c), .. } => Some(c),
            _ => None,
        })
        .collect()
}

#[test]
fn a_late_trade_for_the_previous_order_of_a_level_counts_toward_it() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let first = placed(&sent)[0].clone();
    // Bullet's cancel carries no filled quantity (always 0), then the level is placed again.
    sim.send(&cancelled_with(&first, "0"));
    let second = placed(&sim.send(&book_event()))[0].clone();
    assert_eq!(second.quantity, d("0.01"));
    // The trade for the first order arrives after the next book: 0.004 of it had filled.
    let late = NewOrder { quantity: d("0.004"), ..first.clone() };
    let reply = sim.send(&fill_of(&late));
    let asked = cancels(&reply);
    assert_eq!(asked.len(), 1, "the resting order would overfill the level: {reply:?}");
    assert_eq!(asked[0].client_id, second.client_id);
    // Its own cancel is not the venue's, and the level is placed again for what is left.
    sim.send(&cancelled_with(&second, "0"));
    let rest = placed(&sim.send(&book_event()))[0].clone();
    assert_eq!(rest.quantity, d("0.006"), "0.01 - 0.004");
    assert!(!states(&sim).contains(&LevelState::Held));
}

#[test]
fn a_late_trade_that_completes_the_level_places_it_whole_again() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let first = placed(&sent)[0].clone();
    sim.send(&cancelled_with(&first, "0"));
    let second = placed(&sim.send(&book_event()))[0].clone();
    let reply = sim.send(&fill_of(&first));
    assert_eq!(cancels(&reply).len(), 1);
    sim.send(&cancelled_with(&second, "0"));
    let whole = placed(&sim.send(&book_event()))[0].clone();
    assert_eq!(whole.quantity, d("0.01"));
    assert_eq!(sim.strategy.report().fills, 1);
}

/// The ladder asks to cancel the order resting beside a late fill; the venue refuses the cancel.
fn late_fill_with_cancel_asked(sim: &mut SimHost<Ladder>) -> (NewOrder, NewOrder) {
    let sent = sim.start().unwrap();
    let first = placed(&sent)[0].clone();
    sim.send(&cancelled_with(&first, "0"));
    let second = placed(&sim.send(&book_event()))[0].clone();
    let late = NewOrder { quantity: d("0.004"), ..first.clone() };
    let reply = sim.send(&fill_of(&late));
    assert_eq!(cancels(&reply).len(), 1);
    (first, second)
}

#[test]
fn a_failed_own_cancel_places_nothing_beside_the_resting_order() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let (_, second) = late_fill_with_cancel_asked(&mut sim);
    // The venue refused the cancel: the order still rests, so nothing is placed beside it.
    let cancel = cancel(CancelOrder {
        symbol: second.symbol.clone(),
        order_id: String::new(),
        client_id: second.client_id.clone(),
    });
    sim.send(&outcome(&cancel, Err("venue down")));
    assert_eq!(states(&sim)[0], LevelState::Resting, "still resting");
    sim.market.now_ms += 1_000; // the first wait after a refused cancel
    let next = sim.send(&book_event());
    assert_eq!(placed(&next).len(), 0, "nothing beside the resting order");
    assert_eq!(cancels(&next).len(), 1, "the cancel is asked again");
    // Once it is cancelled the remainder goes out.
    sim.send(&cancelled_with(&second, "0"));
    let rest = placed(&sim.send(&book_event()))[0].clone();
    assert_eq!(rest.quantity, d("0.006"));
}

#[test]
fn a_later_venue_cancel_is_counted_and_its_fill_kept() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let (_, second) = late_fill_with_cancel_asked(&mut sim);
    sim.send(&cancelled_with(&second, "0"));
    let third = placed(&sim.send(&book_event()))[0].clone();
    assert_eq!(third.quantity, d("0.006"));
    // The venue (not the ladder) cancels the remainder order, having filled 0.002 of it.
    sim.market.now_ms += 10_000;
    sim.send(&cancelled_with(&third, "0.002"));
    let again = sim.send(&book_event());
    let again = placed(&again);
    assert_eq!(again[0].quantity, d("0.004"), "0.01 - 0.004 - 0.002");
    assert_eq!(sim.strategy.report().levels[0].filled, d("0.006"));
    assert_eq!(
        sim.strategy.levels[0].cancels.len(),
        2,
        "both venue cancels count; the ladder's own did not"
    );
}

fn cancel_intent(order: &NewOrder) -> Intent {
    cancel(CancelOrder {
        symbol: order.symbol.clone(),
        order_id: String::new(),
        client_id: order.client_id.clone(),
    })
}

#[test]
fn a_refused_place_drops_its_own_cancel() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let (_, second) = late_fill_with_cancel_asked(&mut sim);
    sim.send(&outcome(&cancel_intent(&second), Err("venue down")));
    assert!(sim.strategy.levels[0].cancel_asked());
    // The order the cancel was for turns out to have been refused (not a retried reason): there
    // is nothing to cancel, and nothing to ask again for.
    sim.send(&outcome(&place(second.clone()), Err("risk gate: max position")));
    let rung = &sim.strategy.levels[0];
    assert!(!rung.cancel_asked());
    for _ in 0..5 {
        sim.market.now_ms += 60_000;
        assert_eq!(cancels(&sim.send(&book_event())).len(), 0);
    }
}

#[test]
fn a_refused_cancel_is_asked_again_at_most_once_per_interval() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let (_, second) = late_fill_with_cancel_asked(&mut sim);
    let mut asked = vec![sim.market.now_ms];
    sim.send(&outcome(&cancel_intent(&second), Err("venue down")));
    // A book every 100 ms for 20 s, the venue refusing every cancel.
    for _ in 0..200 {
        sim.market.now_ms += 100;
        let reply = sim.send(&book_event());
        if !cancels(&reply).is_empty() {
            assert_eq!(cancels(&reply).len(), 1);
            asked.push(sim.market.now_ms);
            sim.send(&outcome(&cancel_intent(&second), Err("venue down")));
        }
    }
    let gaps: Vec<u64> = asked.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.len() >= 3, "asked again: {gaps:?}");
    // 1 s, then double each time up to 30 s.
    for (i, gap) in gaps.iter().enumerate() {
        let least = 1_000u64.saturating_mul(1 << i.min(5)).min(30_000);
        assert!(*gap >= least, "gap {i} was {gap} ms, at least {least} ms: {gaps:?}");
    }
}

#[test]
fn a_level_completed_while_its_cancel_is_pending_places_after_the_cancel() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let (_, second) = late_fill_with_cancel_asked(&mut sim);
    // The rest of the resting order fills before its cancel lands: the level is complete.
    let rest = NewOrder { quantity: d("0.006"), ..second.clone() };
    let reply = sim.send(&fill_of(&rest));
    assert_eq!(placed(&reply).len(), 0, "nothing beside the order that may still rest");
    assert_eq!(sim.strategy.report().fills, 1);
    sim.send(&outcome(&cancel_intent(&second), Err("venue down")));
    sim.market.now_ms += 2_000;
    assert_eq!(cancels(&sim.send(&book_event())).len(), 1, "the cancel is still asked for");
    sim.send(&cancelled_with(&second, "0"));
    let whole = sim.send(&book_event());
    let whole = placed(&whole);
    assert_eq!(whole.len(), 1);
    assert_eq!(whole[0].quantity, d("0.01"));
}

/// The whole resting order fills while the ladder's cancel of it is pending: nothing of it rests,
/// the venue refuses the cancel of a filled order, and the level must still be placed again.
#[test]
fn a_current_order_filled_whole_while_its_cancel_is_pending_frees_the_level() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let (_, second) = late_fill_with_cancel_asked(&mut sim);
    sim.send(&fill_of(&second));
    sim.send(&outcome(&cancel_intent(&second), Err("order not found")));
    let mut placed_total = 0;
    for _ in 0..4 {
        sim.market.now_ms += 30_000;
        let r = sim.send(&book_event());
        placed_total += placed(&r).len();
        assert!(cancels(&r).is_empty(), "no cancel for an order that is gone");
    }
    assert_eq!(placed_total, 1, "placed whole once");
    assert_eq!(sim.strategy.report().fills, 1);
}

#[test]
fn a_filled_order_event_frees_the_level_with_its_cancel_pending() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let (_, second) = late_fill_with_cancel_asked(&mut sim);
    let mut filled = cancelled(&second);
    if let Event::Market { event: MarketEvent::Order(o), .. } = &mut filled {
        o.status = OrderStatus::Filled;
        o.filled_quantity = d("0.01");
    }
    sim.send(&filled);
    sim.market.now_ms += 1_000;
    let r = sim.send(&book_event());
    assert_eq!(placed(&r).len(), 1);
    assert_eq!(cancels(&r).len(), 0);
}

/// The rest of the order fills (the level is complete), then the remainder of that same order: the
/// second trade belongs to the old order, not to the level placed next.
#[test]
fn the_rest_then_the_remainder_of_the_order_fill_while_its_cancel_is_pending() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let (_, second) = late_fill_with_cancel_asked(&mut sim);
    let part = |q: &str| fill_of(&NewOrder { quantity: d(q), ..second.clone() });
    assert_eq!(placed(&sim.send(&part("0.006"))).len(), 0, "not beside a resting order");
    sim.send(&part("0.004"));
    sim.send(&outcome(&cancel_intent(&second), Err("order not found")));
    sim.market.now_ms += 30_000;
    let r = sim.send(&book_event());
    let r = placed(&r);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].quantity, d("0.01"), "a whole level: the 0.004 was the old order's");
    assert_eq!(sim.strategy.report().levels[0].filled, d("0"));
}

/// Probe: the late trade completes the level while the current order's cancel is asked, then the
/// current order fills out too. One completion, and the level is placed again whole (not stuck
/// re-cancelling a filled order).
#[test]
fn a_late_trade_completes_the_level_and_then_the_current_order_fills_out() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let first = placed(&sent)[0].clone();
    sim.send(&cancelled_with(&first, "0"));
    let second = placed(&sim.send(&book_event()))[0].clone();
    let late = sim.send(&fill_of(&first));
    assert_eq!(cancels(&late).len(), 1, "the order resting beside a full level is cancelled");
    sim.send(&fill_of(&second));
    assert_eq!(sim.strategy.report().fills, 1, "one completion");
    sim.send(&outcome(&cancel_intent(&second), Err("order not found")));
    let mut total = 0;
    for _ in 0..4 {
        sim.market.now_ms += 30_000;
        let r = sim.send(&book_event());
        total += placed(&r).len();
        assert_eq!(cancels(&r).len(), 0, "no cancel for an order that is gone");
    }
    assert_eq!(total, 1, "placed whole once");
}

/// Probe: a second late trade for an order of a level that has completed and been placed again
/// counts once at most: the earlier cycle's fill is not the new cycle's.
#[test]
fn a_second_late_trade_after_the_level_completed_counts_once() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let first = placed(&sent)[0].clone();
    sim.send(&cancelled_with(&first, "0"));
    let second = placed(&sim.send(&book_event()))[0].clone();
    sim.send(&fill_of(&first));
    sim.send(&cancelled_with(&second, "0"));
    let third = placed(&sim.send(&book_event()))[0].clone();
    assert_eq!(third.quantity, d("0.01"), "a whole level again");
    // Another trade for the first order arrives: it belongs to the cycle that already completed.
    let stray = NewOrder { quantity: d("0.003"), ..first.clone() };
    let r = sim.send(&fill_of(&stray));
    assert_eq!(cancels(&r).len(), 0);
    assert_eq!(sim.strategy.report().fills, 1, "counted once");
    assert_eq!(sim.strategy.report().levels[0].filled, d("0"));
}

/// A stop (the daily loss) refuses every placement for hours while a book arrives every few
/// seconds: the ledger must not grow with the refusals, or the plugin's work grows until it faults
/// instead of resuming when the stop lifts.
#[test]
fn stopped_refusals_keep_the_ledger_bounded() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    sim.refuse = |_| Some("risk gate: new orders are stopped: the daily loss limit was hit");
    sim.start().unwrap();
    for _ in 0..5_000 {
        sim.market.now_ms += 3_000;
        sim.send(&book_event());
    }
    for level in &sim.strategy.levels {
        assert!(level.order_count() <= level::MAX_ORDERS, "{}", level.order_count());
    }
    // The stop lifts: the levels are placed again, whole.
    sim.refuse = |_| None;
    sim.market.now_ms += 3_000;
    assert_eq!(placed(&sim.send(&book_event())).len(), 2);
    assert!(states(&sim).iter().all(|s| *s == LevelState::Resting));
}

#[test]
fn sweeps_and_crossing_refusals_keep_the_ledger_bounded_too() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    sim.refuse = crossing_buys;
    sim.start().unwrap();
    sim.market.set_book(Some(bb_strategy_sdk::testing::book("70", "72", "10")));
    for i in 0..3_000 {
        sim.market.now_ms += 3_000;
        sim.market.set_book(Some(if i % 2 == 0 {
            bb_strategy_sdk::testing::book("70", "72", "10")
        } else {
            bb_strategy_sdk::testing::book("75", "76", "10")
        }));
        sim.send(&book_event());
    }
    for level in &sim.strategy.levels {
        assert!(level.order_count() <= level::MAX_ORDERS, "{}", level.order_count());
    }
}

fn rejected(order: &NewOrder) -> Event {
    let mut event = cancelled(order);
    if let Event::Market { event: MarketEvent::Order(o), .. } = &mut event {
        o.status = OrderStatus::Rejected;
    }
    event
}

/// The venue rejects a placed order on a book that does not cross it (a reason the ladder cannot
/// read): placed again on a later usable book, under the same cap as a rate refusal.
#[test]
fn a_rejection_on_a_non_crossing_book_is_retried_under_the_cap() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let mut order = placed(&sent)[0].clone();
    for round in 0..3 {
        sim.send(&rejected(&order));
        sim.market.now_ms += 61_000;
        let r = sim.send(&book_event());
        let r = placed(&r);
        assert_eq!(r.len(), 1, "retry {round}");
        order = r[0].clone();
    }
    // Rejected once more, a rate window on: past the cap the level is held.
    sim.send(&rejected(&order));
    sim.market.now_ms += 61_000;
    assert_eq!(placed(&sim.send(&book_event())).len(), 0, "held");
    assert_eq!(states(&sim)[0], LevelState::Held);
}

/// Bullet's broker answers an HTTP error as a refusal even when the order may have committed: the
/// order is kept in the ledger (an unreadable refusal is final), so a trade for it still counts.
#[test]
fn a_trade_for_an_order_refused_with_an_unreadable_error_still_counts() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    sim.refuse = |o| (o.side == Side::Buy).then_some("request failed: http 502");
    let sent = sim.start().unwrap();
    let buy = placed(&sent)[0].clone();
    assert_eq!(states(&sim)[0], LevelState::Refused, "refused for good");
    // It had committed after all, and fills.
    sim.send(&fill_of(&buy));
    assert_eq!(sim.strategy.report().fills, 1, "the completion is not lost");
    // Nothing else is placed on a level whose placement may have committed (it is final).
    sim.refuse = |_| None;
    sim.market.now_ms += 1_000;
    let again = sim.send(&book_event());
    assert!(placed(&again).iter().all(|o| o.side == Side::Sell), "no second buy beside it");
    assert_eq!(states(&sim)[0], LevelState::Refused);
}
