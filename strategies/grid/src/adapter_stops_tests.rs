use super::*;

// --- Rule 2: the hard stop. ---

fn closes(intents: &[Intent]) -> Vec<(Side, Decimal, bool, OrderType)> {
    placements(intents).iter().map(|o| (o.side, o.quantity, o.reduce_only, o.order_type)).collect()
}

#[test]
fn past_the_hard_stop_it_cancels_its_own_orders_closes_reduce_only_and_stops() {
    let (mut grid, _) = running(test_config());
    let two = d("2");
    // 72.1 is above the stop at 72: nothing yet.
    let near = book("72.0", "72.2");
    let out = grid.on_event(&Event::Book, &at(Some(&near), &[], NOON, two));
    assert!(out.is_empty(), "{out:?}");
    // 71.9: cancel its own orders (the owner's on the market stay: a normal stop) and sell the 2
    // held, reduce-only, at most 20 bps past the bid.
    let past = book("71.8", "72.0");
    let out = grid.on_event(&Event::Book, &at(Some(&past), &[], NOON, two));
    assert!(matches!(out[0], Intent::CancelOwn), "{out:?}");
    assert_eq!(closes(&out), [(Side::Sell, two, true, OrderType::Market)]);
    assert!(report(&grid)["stopped"].as_str().unwrap().contains("hard stop"));
    // Still holding on the next tick: close again, wider; never a level.
    let back = mid_book();
    let out = grid.on_event(&Event::Tick, &at(Some(&back), &[], NOON + 5_000, d("0.5")));
    assert_eq!(closes(&out), [(Side::Sell, d("0.5"), true, OrderType::Market)]);
    assert!(placements(&out)[0].price < d("75.4"), "past the bid: {:?}", placements(&out)[0].price);
    // Flat: nothing more, ever.
    let out = grid.on_event(&Event::Tick, &at(Some(&back), &[], NOON + 10_000, Decimal::ZERO));
    assert!(out.is_empty(), "{out:?}");
    let fill = Event::Fill(trade(Some("x"), Side::Buy, "75"));
    assert!(grid.on_event(&fill, &view(Some(&back), &[], NOON)).is_empty());
}

#[test]
fn the_short_sides_hard_stop_is_above_the_range_and_buys_back() {
    let (mut grid, _) = running(test_config());
    let past = book("80.0", "80.2");
    let out = grid.on_event(&Event::Book, &at(Some(&past), &[], NOON, d("-3")));
    assert_eq!(closes(&out), [(Side::Buy, d("3"), true, OrderType::Market)]);
    // A long grid has no stop above: the price above its range is its profitable side.
    let long = with(test_config(), serde_json::json!({ "direction": "long" }));
    let (mut grid, _) = running(long);
    let out = grid.on_event(&Event::Book, &at(Some(&past), &[], NOON, Decimal::ZERO));
    assert!(out.is_empty(), "{out:?}");
}

/// The hard stop holds the instance once and says why; a daily loss trip never asks.
#[test]
fn the_hard_stop_holds_the_instance_and_says_why() {
    let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = Arc::clone(&asked);
    let (grid, _) = running(test_config());
    let mut grid = grid.with_hard_stop_hold(Arc::new(move || {
        count.fetch_add(1, Ordering::SeqCst);
    }));
    let past = book("71.8", "72.0");
    let out = grid.on_event(&Event::Book, &at(Some(&past), &[], NOON, d("2")));
    assert_eq!(asked.load(Ordering::SeqCst), 1, "the instance is held");
    let said = out.iter().find_map(|i| match i {
        Intent::Notify(bb_strategy_sdk::Alert::Raise { text, .. }) => Some(text.clone()),
        _ => None,
    });
    let said = said.expect("the owner is told");
    assert!(said.contains("hard stop") && said.contains("resume"), "{said}");
    let out = grid.on_event(&Event::Tick, &at(Some(&mid_book()), &[], NOON + 5_000, d("2")));
    assert_eq!(closes(&out), [(Side::Sell, d("2"), true, OrderType::Market)], "it closes again");
    assert_eq!(asked.load(Ordering::SeqCst), 1, "asked once");

    let tripped = Arc::new(AtomicBool::new(false));
    let probe = Arc::clone(&tripped);
    let (grid, _) = running(test_config());
    let mut grid = grid
        .with_day_stop(Arc::new(move || probe.load(Ordering::SeqCst)))
        .with_hard_stop_hold(Arc::new(|| panic!("the loss watch holds a daily loss trip")));
    tripped.store(true, Ordering::SeqCst);
    let out = grid.on_event(&Event::Tick, &at(Some(&mid_book()), &[], NOON, d("2")));
    assert!(matches!(out[0], Intent::CancelAll), "{out:?}");

    // The owner's resume starts a new grid, which ends the hard stop's alert.
    let mut resumed = GridStrategy::new(test_config()).with_hard_stop_hold(Arc::new(|| {}));
    let start = resumed.start(&view(Some(&mid_book()), &[], NOON)).unwrap();
    let resolved = start.iter().any(|i| {
        matches!(i, Intent::Notify(bb_strategy_sdk::Alert::Resolve { key }) if key == crate::KEY_HARD_STOP)
    });
    assert!(resolved, "{start:?}");
}

// --- Rule 4: the daily loss stop. ---

#[test]
fn the_daily_loss_stop_flattens_and_stops_the_grid() {
    let tripped = Arc::new(AtomicBool::new(false));
    let probe = Arc::clone(&tripped);
    let grid = GridStrategy::new(test_config())
        .with_day_stop(Arc::new(move || probe.load(Ordering::SeqCst)));
    let (mut grid, _) = running_on(grid, &mid_book());
    let b = mid_book();
    assert!(
        placements(&grid.on_event(&Event::Tick, &at(Some(&b), &[], NOON, d("1"))))
            .iter()
            .all(|o| !o.reduce_only)
    );
    tripped.store(true, Ordering::SeqCst);
    let out = grid.on_event(&Event::Tick, &at(Some(&b), &[], NOON + 5_000, d("1")));
    // The daily loss stop is a safety stop: every order on the market goes, as the path's halt.
    assert!(matches!(out[0], Intent::CancelAll), "{out:?}");
    assert_eq!(closes(&out), [(Side::Sell, d("1"), true, OrderType::Market)]);
    assert!(report(&grid)["stopped"].as_str().unwrap().contains("daily loss"));
    // Winding down with a position left closes it too.
    let out = grid.stop(&at(Some(&b), &[], NOON + 6_000, d("1")));
    assert_eq!(closes(&out), [(Side::Sell, d("1"), true, OrderType::Market)]);
}

/// The host's daily loss stop, shown in the view, stops the grid as the build's closure does; any
/// other stop of the host's (a pause, a stale price) leaves the grid to hold, as the path does.
#[test]
fn the_hosts_daily_loss_stop_in_the_view_flattens_and_stops_the_grid() {
    use bb_strategy_sdk::stops::StopReason;
    let (mut grid, _) = running(test_config());
    let b = mid_book();
    let paused = StopReason::Paused;
    let held = grid
        .on_event(&Event::Tick, &View { stop: Some(&paused), ..at(Some(&b), &[], NOON, d("1")) });
    assert!(held.iter().all(|i| !matches!(i, Intent::CancelAll)), "{held:?}");
    let loss = StopReason::DailyLoss { day: bb_strategy_sdk::stops::utc_day(NOON) };
    let out = grid.on_event(
        &Event::Tick,
        &View { stop: Some(&loss), ..at(Some(&b), &[], NOON + 5_000, d("1")) },
    );
    assert!(matches!(out[0], Intent::CancelAll), "{out:?}");
    assert_eq!(closes(&out), [(Side::Sell, d("1"), true, OrderType::Market)]);
    assert!(report(&grid)["stopped"].as_str().unwrap().contains("daily loss"));
}

/// A daily loss stop's cancel that failed is asked again market-wide, a hard stop's as its own.
#[test]
fn a_failed_daily_loss_cancel_is_asked_again_market_wide() {
    let tripped = Arc::new(AtomicBool::new(true));
    let probe = Arc::clone(&tripped);
    let grid = GridStrategy::new(test_config())
        .with_day_stop(Arc::new(move || probe.load(Ordering::SeqCst)));
    let (mut grid, _) = running_on(grid, &mid_book());
    let b = mid_book();
    let out = grid.on_event(&Event::Tick, &at(Some(&b), &[], NOON, Decimal::ZERO));
    assert!(matches!(out.as_slice(), [Intent::CancelAll]), "{out:?}");
    let failed = Event::Failed { intent: Intent::CancelAll, error: "transport".into() };
    grid.on_event(&failed, &at(Some(&b), &[], NOON + 1_000, Decimal::ZERO));
    let again = grid.on_event(&Event::Tick, &at(Some(&b), &[], NOON + 6_000, Decimal::ZERO));
    assert!(matches!(again.as_slice(), [Intent::CancelAll]), "{again:?}");
}

/// The grid passed its hard stop flat: it asked to cancel its own orders, and closes nothing.
fn tripped() -> (GridStrategy, OrderBook) {
    let (mut grid, _) = running(test_config());
    let past = book("71.8", "72.0");
    let out = grid.on_event(&Event::Book, &view(Some(&past), &[], NOON));
    assert!(matches!(out.as_slice(), [Intent::CancelOwn]), "{out:?}");
    (grid, past)
}

fn cancel_all_answer(success: bool) -> Event {
    let error = (!success).then(|| "venue down".to_string());
    Event::Outcome {
        intent: Intent::CancelOwn,
        result: OrderResult { order_id: None, client_id: None, success, error },
    }
}

/// The stop's cancel-all failed: orders may still rest, so it is asked again on every tick until
/// one lands.
#[test]
fn a_failed_cancel_all_is_asked_again_on_the_next_tick_until_it_lands() {
    let (mut grid, past) = tripped();
    assert!(
        grid.on_event(&cancel_all_answer(false), &view(Some(&past), &[], NOON + 1_000)).is_empty()
    );
    let again = grid.on_event(&Event::Tick, &view(Some(&past), &[], NOON + 6_000));
    assert!(matches!(again.as_slice(), [Intent::CancelOwn]), "{again:?}");
    grid.on_event(&cancel_all_answer(false), &view(Some(&past), &[], NOON + 6_500));
    let third = grid.on_event(&Event::Tick, &view(Some(&past), &[], NOON + 11_000));
    assert!(matches!(third.as_slice(), [Intent::CancelOwn]), "{third:?}");
    // Once it lands the stopped grid stays quiet.
    grid.on_event(&cancel_all_answer(true), &view(Some(&past), &[], NOON + 11_500));
    assert!(grid.on_event(&Event::Tick, &view(Some(&past), &[], NOON + 16_000)).is_empty());
}

#[test]
fn a_cancel_all_that_failed_as_a_whole_is_asked_again_too() {
    let (mut grid, past) = tripped();
    let failed = Event::Failed { intent: Intent::CancelOwn, error: "transport".into() };
    assert!(grid.on_event(&failed, &view(Some(&past), &[], NOON + 1_000)).is_empty());
    let again = grid.on_event(&Event::Tick, &view(Some(&past), &[], NOON + 6_000));
    assert!(matches!(again.as_slice(), [Intent::CancelOwn]), "{again:?}");
}

#[test]
fn the_owner_is_told_after_five_failed_cancel_alls_in_a_row_and_the_grid_keeps_trying() {
    let (mut grid, past) = tripped();
    let raised = |i: &Intent| matches!(i, Intent::Notify(bb_strategy_sdk::Alert::Raise { key, .. }) if key == "grid_cancel_all_failing");
    let mut alerts = Vec::new();
    let mut raised_at = Vec::new();
    for n in 1..=7_u64 {
        let out =
            grid.on_event(&cancel_all_answer(false), &view(Some(&past), &[], NOON + n * 5_000));
        if out.iter().any(raised) {
            raised_at.push(n);
        }
        alerts.extend(out);
        let again = grid.on_event(&Event::Tick, &view(Some(&past), &[], NOON + 2_000 + n * 5_000));
        assert!(matches!(again.as_slice(), [Intent::CancelOwn]), "keeps trying: {again:?}");
    }
    assert_eq!(alerts.iter().filter(|i| raised(i)).count(), 1, "told once: {alerts:?}");
    assert_eq!(raised_at, [5], "and at the fifth failure, not before or after");
    // It lands: the alert is resolved.
    let out = grid.on_event(&cancel_all_answer(true), &view(Some(&past), &[], NOON + 200_000));
    assert!(
        matches!(out.as_slice(), [Intent::Notify(bb_strategy_sdk::Alert::Resolve { .. })]),
        "{out:?}"
    );
}

/// The venue holds the grid's hard stop for its position: below the range for a long, above it
/// for a short, none while flat.
#[test]
fn the_stop_the_venue_holds_is_the_hard_stop() {
    let (grid, _) = running(test_config());
    let book = mid_book();
    let stop = |position: &str| grid.protective_stop(&at(Some(&book), &[], NOON, d(position)));
    assert_eq!(stop("2").and_then(|p| p.stop), Some(d("72")), "the long side's, at 72");
    assert_eq!(stop("-3").and_then(|p| p.stop), Some(d("80")), "the short side's, at 80");
    assert_eq!(stop("0"), None);
}
