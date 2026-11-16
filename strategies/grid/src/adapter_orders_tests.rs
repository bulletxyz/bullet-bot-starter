use super::*;

#[test]
fn a_level_the_venue_still_lists_stays_active_and_is_not_replaced() {
    let (mut grid, orders) = running(test_config());
    let resting = resting(&orders);
    let out = grid.on_event(&Event::Tick, &view(Some(&mid_book()), &resting, NOON));
    assert!(placements(&out).is_empty(), "{out:?}");
    assert!(levels(&grid).iter().all(|(state, _)| state == "Active"));
}

/// A place the venue left unanswered may have landed: its level is held, not placed again under a
/// new id (a landed first order would be a stray whose fills nobody tracks).
#[test]
fn an_unknown_place_holds_its_grid_level() {
    let mut grid = GridStrategy::new(test_config());
    let b = mid_book();
    let intents = grid.start(&view(Some(&b), &[], NOON)).unwrap();
    unknown_all(&mut grid, &intents, &b);
    assert!(levels(&grid).iter().all(|(state, _)| state == "Active"), "{:?}", levels(&grid));
    // A tick with no venue list (nothing read yet) places nothing.
    let unread = View { open_orders: None, positions: None, ..view(Some(&b), &[], NOON) };
    let out = grid.on_event(&Event::Tick, &unread);
    assert!(placements(&out).is_empty(), "{out:?}");
}

/// A read just after the unknown place may lag it: the level is freed only by a later one.
#[test]
fn an_unknown_level_is_freed_only_by_a_read_after_the_settle_margin() {
    let mut grid = GridStrategy::new(test_config());
    let b = mid_book();
    let intents = grid.start(&view(Some(&b), &[], NOON)).unwrap();
    unknown_all(&mut grid, &intents, &b);
    let early = grid.on_event(&Event::Tick, &view(Some(&b), &[], NOON + 500));
    assert!(placements(&early).is_empty(), "inside the margin the empty read is not believed");
    let late = grid.on_event(&Event::Tick, &view(Some(&b), &[], NOON + 3_000));
    assert_eq!(placements(&late).len(), 5, "a read after it omits the orders: re-placed");
}

/// The read's own start time decides, not when it reached the grid: a read that started inside the
/// margin, delivered late, does not free the level; and a level the venue lists is no longer held.
#[test]
fn an_unknown_level_is_freed_by_the_reads_start_not_its_delivery() {
    let mut grid = GridStrategy::new(test_config());
    let b = mid_book();
    let intents = grid.start(&view(Some(&b), &[], NOON)).unwrap();
    unknown_all(&mut grid, &intents, &b);
    let late = View { read_started_ms: Some(NOON + 500), ..view(Some(&b), &[], NOON + 3_000) };
    assert!(placements(&grid.on_event(&Event::Tick, &late)).is_empty(), "the read began too early");
    let placed: Vec<NewOrder> = placements(&intents).into_iter().cloned().collect();
    grid.on_event(&Event::Tick, &view(Some(&b), &resting(&placed), NOON + 4_000));
    assert!(grid.placed_at.is_empty(), "listed orders are no longer unknown");
}

// --- A level whose Place is in flight (sent, not answered yet) holds that order. ---

fn failed(order: &NewOrder) -> Event {
    Event::Failed { intent: Intent::Place(order.clone()), error: "transport".into() }
}

fn cid_at(orders: &[NewOrder], price: &str) -> String {
    orders.iter().find(|o| o.price == d(price)).and_then(|o| o.client_id.clone()).unwrap()
}

/// The host defers the re-armed Place behind a venue read, so a second fill lands while it is in flight.
#[test]
fn a_level_in_flight_is_not_placed_again_by_the_next_pass() {
    // Anchor 76 empties 76: buys 74, 75, sells 77, 78.
    let (mut grid, orders) =
        running(with(test_config(), serde_json::json!({ "anchor_price": "76" })));
    let b = mid_book();
    let first = Event::Fill(trade(Some(&cid_at(&orders, "75")), Side::Buy, "75"));
    let out = grid.on_event(&first, &at(Some(&b), &[], NOON, d("1")));
    let rearmed: Vec<String> = placements(&out).iter().map(|o| o.price.to_string()).collect();
    assert_eq!(rearmed, ["76"], "the filled buy re-arms a sell one level up");
    let second = Event::Fill(trade(Some(&cid_at(&orders, "74")), Side::Buy, "74"));
    let out = grid.on_event(&second, &at(Some(&b), &[], NOON + 100, d("2")));
    let again: Vec<String> = placements(&out).iter().map(|o| o.price.to_string()).collect();
    assert_eq!(again, ["75"], "only the newly re-armed level: 76 is in flight");
}

#[test]
fn an_in_flight_buy_counts_toward_the_long_cap() {
    // Buys 74, 75, sells 77, 78: a worst case of two coins each side, the cap.
    let cfg = with(test_config(), serde_json::json!({ "anchor_price": "76", "max_position": "2" }));
    let mut grid = GridStrategy::new(cfg);
    let b = mid_book();
    let intents = grid.start(&view(Some(&b), &[], NOON)).unwrap();
    let sent: Vec<NewOrder> = placements(&intents).into_iter().cloned().collect();
    // Every answer but the buy at 75's is back.
    for order in sent.iter().filter(|o| o.price != d("75")) {
        grid.on_event(&acked(order), &view(Some(&b), &[], NOON));
    }
    // The venue no longer lists the buy at 74: it goes back to be placed. Long 1, with the buy at
    // 75 in flight, one more buy would make 3.
    let listed: Vec<NewOrder> = sent.iter().filter(|o| o.side == Side::Sell).cloned().collect();
    let out = grid.on_event(&Event::Tick, &at(Some(&b), &resting(&listed), NOON + 3_000, d("1")));
    assert!(placements(&out).is_empty(), "{out:?}");
}

#[test]
fn a_failed_place_frees_its_level_for_the_next_pass() {
    let mut grid = GridStrategy::new(test_config());
    let b = mid_book();
    let intents = grid.start(&view(Some(&b), &[], NOON)).unwrap();
    let order = placements(&intents)[1].clone();
    assert!(grid.on_event(&failed(&order), &view(Some(&b), &[], NOON)).is_empty());
    assert_eq!(levels(&grid)[1], ("Pending".to_string(), false));
    let unread = View { open_orders: None, positions: None, ..view(Some(&b), &[], NOON + 1_000) };
    let out = grid.on_event(&Event::Tick, &unread);
    let again: Vec<String> = placements(&out).iter().map(|o| o.price.to_string()).collect();
    assert_eq!(again, [order.price.to_string()], "{out:?}");
}

/// It does not trade a long or short plan without its starting position.
#[test]
fn a_starting_ioc_that_was_never_sent_stops_the_grid() {
    let cfg = with(test_config(), serde_json::json!({ "direction": "long" }));
    let mut grid = GridStrategy::new(cfg);
    let b = mid_book();
    let intents = grid.start(&view(Some(&b), &[], NOON)).unwrap();
    let ioc = placements(&intents)[0].clone();
    assert_eq!(ioc.order_type, OrderType::Market);
    let out = grid.on_event(&failed(&ioc), &view(Some(&b), &[], NOON));
    assert!(
        matches!(out.as_slice(), [Intent::Shutdown(why)] if why.contains("transport")),
        "{out:?}"
    );
}

/// Here the hard stop clears every level while the Place is in flight.
#[test]
fn an_order_answered_after_its_level_let_go_is_cancelled() {
    let mut grid = GridStrategy::new(test_config());
    let b = mid_book();
    let intents = grid.start(&view(Some(&b), &[], NOON)).unwrap();
    let order = placements(&intents)[0].clone();
    let past = book("71.8", "72.0");
    let out = grid.on_event(&Event::Book, &view(Some(&past), &[], NOON + 100));
    // Flat: its own orders pulled, nothing to close; the instance stood down and the owner told.
    assert!(
        matches!(out.as_slice(), [Intent::CancelOwn, Intent::StandDown(_), Intent::Notify(_)]),
        "{out:?}"
    );
    let out = grid.on_event(&acked(&order), &view(Some(&past), &[], NOON + 200));
    assert_eq!(cancels(&out), [order.client_id.clone().unwrap()], "{out:?}");
    // A refusal leaves nothing resting: nothing to cancel.
    let refused = Event::Outcome {
        intent: Intent::Place(placements(&intents)[1].clone()),
        result: OrderResult { order_id: None, client_id: None, success: false, error: None },
    };
    assert!(grid.on_event(&refused, &view(Some(&past), &[], NOON + 300)).is_empty());
}

// --- A level lets go of its order only on proof it is gone; a stray it left is cancelled. ---

/// The venue's REST can lag an acked placement, so the order's fill must still find its level.
#[test]
fn an_acked_level_is_freed_only_by_a_read_after_the_settle_margin() {
    // Anchor 76 empties 76: buys 74, 75, sells 77, 78.
    let (mut grid, orders) =
        running(with(test_config(), serde_json::json!({ "anchor_price": "76" })));
    let b = mid_book();
    let early = grid.on_event(&Event::Tick, &view(Some(&b), &[], NOON + 500));
    assert!(placements(&early).is_empty(), "inside the margin the empty read is not believed");
    let fill = Event::Fill(trade(Some(&cid_at(&orders, "75")), Side::Buy, "75"));
    let out = grid.on_event(&fill, &at(Some(&b), &[], NOON + 600, d("1")));
    assert_eq!(prices_of(&out, Side::Sell), ["76"], "the fill found its level: {out:?}");
    let late = grid.on_event(&Event::Tick, &at(Some(&b), &[], NOON + 3_000, d("1")));
    assert_eq!(prices_of(&late, Side::Buy), ["74"], "a read after the margin frees the rest");
}

/// A failed pause cancel leaves the order resting with no level; the owner's order beside it is never cancelled.
#[test]
fn a_stray_a_failed_pause_cancel_left_is_cancelled_by_a_later_read() {
    let long = with(test_config(), serde_json::json!({ "direction": "long" }));
    let (mut grid, orders) = running(long);
    let listed = resting(&orders);
    let above = book("78.5", "78.7");
    let out = grid.on_event(&Event::Tick, &at(Some(&above), &listed, NOON + 3_000, d("0")));
    let paused = cancels(&out);
    assert_eq!(paused.len(), 2, "the two buys: {out:?}");
    for cancel in out.iter().filter(|i| matches!(i, Intent::Cancel(_))) {
        let failed = Event::Failed { intent: cancel.clone(), error: "transport".into() };
        grid.on_event(&failed, &view(Some(&above), &[], NOON + 3_100));
    }
    let owners = Order { client_id: Some("7".into()), ..listed[1].clone() };
    let mut still = listed.clone();
    still.push(owners);
    let soon = grid.on_event(&Event::Tick, &at(Some(&above), &still, NOON + 4_000, d("0")));
    assert!(cancels(&soon).is_empty(), "inside the margin the cancel may still land: {soon:?}");
    let later = grid.on_event(&Event::Tick, &at(Some(&above), &still, NOON + 6_000, d("0")));
    assert_eq!(cancels(&later), paused, "only the strays: {later:?}");
    // Gone from a read past the margin: forgotten, nothing more to cancel.
    let gone = grid.on_event(&Event::Tick, &at(Some(&above), &[], NOON + 9_000, d("0")));
    assert!(cancels(&gone).is_empty(), "{gone:?}");
}
