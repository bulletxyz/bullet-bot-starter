//! `instance.on_shutdown = keep` with the grid (money safety): a start that reclaims what a kept
//! shutdown left takes over the position (the host cancelled its orders) and lays its levels again
//! within its worst case on each side; a stopped grid closes at a kept shutdown.

use super::*;

/// One of the bot's own orders a kept run left resting on the venue.
fn kept(id: &str, side: Side, price: &str) -> Order {
    Order {
        id: id.into(),
        client_id: Some(bb_strategy_support::ClientIdIssuer::own(NOON - 60_000).issue()),
        symbol: "BTC-USD".into(),
        side,
        order_type: OrderType::Limit,
        price: d(price),
        quantity: d("1"),
        filled_quantity: Decimal::ZERO,
        status: OrderStatus::Open,
    }
}

/// The run before held a long of one level: the buy at 75 filled, so 74 still rests as a buy and
/// 77, 78 as sells (76 was re-armed and is to be placed again).
fn left() -> Vec<Order> {
    vec![
        kept("k-74", Side::Buy, "74"),
        kept("k-77", Side::Sell, "77"),
        kept("k-78", Side::Sell, "78"),
    ]
}

/// A reclaiming start takes the kept long as its own and lays its levels again: the long side holds
/// two levels (74, 75), the kept long fills one, so one buy goes out with the sells, and no entry.
#[test]
fn a_reclaiming_start_takes_over_the_position_and_lays_its_levels_again() {
    let mut grid = GridStrategy::new(test_config());
    grid.reclaim();
    let b = mid_book();
    let intents =
        grid.start(&at(Some(&b), &[], NOON, d("1")).view()).expect("a kept long is its own");
    let placed = placements(&intents);
    let buys = placed.iter().filter(|o| o.side == Side::Buy).count();
    assert_eq!(buys, 1, "the worst case leaves one buy: {intents:?}");
    assert_eq!(placed.iter().filter(|o| o.side == Side::Sell).count(), 3, "{intents:?}");
    assert!(placed.iter().all(|o| o.order_type != OrderType::Market), "no entry");
}

/// Without a reclaim (a start under `close`) a position is still refused, as before.
#[test]
fn a_start_that_does_not_reclaim_still_refuses_a_position() {
    let b = mid_book();
    let e = GridStrategy::new(test_config()).start(&at(Some(&b), &left(), NOON, d("1")).view());
    assert!(e.is_err());
}

/// A stopped grid (past its hard stop) closes at a kept shutdown too.
#[test]
fn a_stopped_grid_may_not_keep() {
    let (mut grid, _) = running(test_config());
    assert!(grid.may_keep());
    let crash = book("71", "71.2");
    grid.on_event(&book_event(), &at(Some(&crash), &[], NOON, d("2")).view());
    assert!(!grid.may_keep(), "past the hard stop it closes");
}
/// level goes out.
#[test]
fn a_reclaiming_start_under_the_daily_loss_stop_stops_and_closes_at_once() {
    let mut grid = GridStrategy::new(test_config()).with_day_stop(Arc::new(|| true));
    grid.reclaim();
    let b = mid_book();
    let intents = grid.start(&at(Some(&b), &left(), NOON, d("1")).view()).unwrap();
    assert!(intents.iter().any(|i| *i == cancel_all()), "{intents:?}");
    let closes: Vec<_> = placements(&intents).into_iter().filter(|o| o.reduce_only).collect();
    assert_eq!(closes.len(), 1, "{intents:?}");
    assert_eq!(placements(&intents).len(), 1, "no level placed");
    assert!(!grid.may_keep());
}

/// A reclaimed grid never holds more than its plan's worst case on a side: the kept position and
/// every resting order on that side count, so a level the run before filled is not laid again
/// until the position comes back down.
#[test]
fn a_reclaimed_grid_is_held_to_its_worst_case_on_each_side() {
    let mut grid = GridStrategy::new(test_config());
    grid.reclaim();
    let b = mid_book();
    // Long two levels already, nothing kept: no buy may go out at all.
    let intents = grid.start(&at(Some(&b), &[], NOON, d("2")).view()).unwrap();
    let buys = placements(&intents).into_iter().filter(|o| o.side == Side::Buy).count();
    assert_eq!(buys, 0, "{intents:?}");
    // Back down to one: one buy may rest again.
    let later = grid.on_event(&Event::Tick, &at(Some(&b), &[], NOON + 1_000, d("1")).view());
    let buys = placements(&later).into_iter().filter(|o| o.side == Side::Buy).count();
    assert_eq!(buys, 1, "{later:?}");
}
