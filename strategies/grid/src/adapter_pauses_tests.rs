use super::*;

// --- Rule 3: out of range. ---

#[test]
fn a_long_grid_above_its_range_cancels_its_buys_and_resumes_back_inside() {
    let long = with(test_config(), serde_json::json!({ "direction": "long" }));
    let (mut grid, orders) = running(long);
    let buys: Vec<String> = orders
        .iter()
        .filter(|o| o.side == Side::Buy && o.order_type != OrderType::Market)
        .filter_map(|o| o.client_id.clone())
        .collect();
    assert_eq!(buys.len(), 2);
    // Flat above 78: every resting order opens. The buys are cancelled.
    let above = book("78.5", "78.7");
    let out = grid.on_event(&Event::Tick, &at(Some(&above), &[], NOON, Decimal::ZERO));
    let gone = cancels(&out);
    assert!(buys.iter().all(|b| gone.contains(b)), "{gone:?} {buys:?}");
    assert!(placements(&out).is_empty(), "{out:?}");
    assert_eq!(report(&grid)["paused"], true);
    // Back inside: the paused levels are placed again.
    let out = grid.on_event(&Event::Tick, &at(Some(&mid_book()), &[], NOON + 5_000, Decimal::ZERO));
    let again: Vec<Side> = placements(&out).iter().map(|o| o.side).collect();
    assert!(again.iter().filter(|s| **s == Side::Buy).count() >= 2, "{again:?}");
    assert_eq!(report(&grid)["paused"], false);
}

#[test]
fn below_the_range_but_above_the_stop_the_grid_holds() {
    let long = with(test_config(), serde_json::json!({ "direction": "long" }));
    let (mut grid, _) = running(long);
    // 73 is under the range (74) and over the stop (72): the losing side, held.
    let below = book("72.9", "73.1");
    let resting: Vec<Order> = Vec::new();
    let out = grid.on_event(&Event::Tick, &at(Some(&below), &resting, NOON, d("5")));
    assert!(
        cancels(&out).is_empty() && !out.iter().any(|i| matches!(i, Intent::CancelAll)),
        "{out:?}"
    );
    assert!(report(&grid)["stopped"].is_null());
    assert_eq!(report(&grid)["paused"], false);
}

// --- Rule 4: the trend filter and the daily loss stop. ---

#[test]
fn a_trend_filter_trip_cancels_the_opening_orders_and_keeps_the_closing_ones() {
    let cfg = with(
        test_config(),
        serde_json::json!({ "trend_filter": { "fast_secs": 30, "slow_secs": 600, "pause_divergence_bps": "50" } }),
    );
    let (mut grid, orders) = running(cfg);
    let resting = resting(&orders);
    let calm = book("75.4", "75.6");
    grid.on_event(&Event::Tick, &view(Some(&calm), &resting, NOON));
    // Long 1 (the buy at 75 filled): the sell at 76 closes it and stays; the rest are cancelled.
    let jump = book("76.9", "77.1");
    let out = grid.on_event(&Event::Tick, &at(Some(&jump), &resting, NOON + 60_000, d("1")));
    let gone = cancels(&out);
    let cid = |price: &str| {
        orders.iter().find(|o| o.price == d(price)).and_then(|o| o.client_id.clone()).unwrap()
    };
    assert!(!gone.contains(&cid("76")), "the closing sell stays: {gone:?}");
    for opening in ["74", "75", "77", "78"] {
        assert!(gone.contains(&cid(opening)), "{opening} cancelled: {gone:?}");
    }
    assert!(!out.iter().any(|i| matches!(i, Intent::CancelAll)), "{out:?}");
    assert_eq!(report(&grid)["paused"], true);
}

fn trending_long() -> GridStrategy {
    let cfg = with(
        test_config(),
        serde_json::json!({ "direction": "long", "trend_filter": {
            "fast_secs": 30, "slow_secs": 600, "pause_divergence_bps": "200" } }),
    );
    running(cfg).0
}

fn tick_buys(grid: &mut GridStrategy, b: &OrderBook, ms: u64) -> usize {
    let out = grid.on_event(&Event::Tick, &at(Some(b), &[], ms, Decimal::ZERO));
    placements(&out).iter().filter(|o| o.side == Side::Buy).count()
}

#[test]
fn a_trend_that_clears_out_of_range_keeps_the_opening_orders_until_the_price_is_back() {
    let mut grid = trending_long();
    let (above, inside) = (book("78.9", "79.1"), book("77.8", "78.0"));
    tick_buys(&mut grid, &above, NOON + 60_000);
    assert!(grid.state.paused && grid.range_paused);
    // The trend settles while the price stays above: the range still holds the buys.
    assert_eq!(tick_buys(&mut grid, &above, NOON + 7_200_000), 0);
    assert!(!grid.state.paused);
    assert_eq!(report(&grid)["paused"], true);
    // Back inside without a new trend: the buys go out again.
    assert!(tick_buys(&mut grid, &inside, NOON + 7_260_000) >= 2);
    assert_eq!(report(&grid)["paused"], false);
}

#[test]
fn back_in_range_while_the_trend_holds_keeps_the_opening_orders_until_it_clears() {
    let mut grid = trending_long();
    let (above, inside) = (book("78.9", "79.1"), book("77.8", "78.0"));
    tick_buys(&mut grid, &above, NOON + 60_000);
    assert!(grid.state.paused && grid.range_paused);
    // Back inside, the trend still tripped: the trend holds the buys.
    assert_eq!(tick_buys(&mut grid, &inside, NOON + 120_000), 0);
    assert!(!grid.range_paused && grid.state.paused);
    assert_eq!(report(&grid)["paused"], true);
    // The trend settles: the buys go out again.
    assert!(tick_buys(&mut grid, &inside, NOON + 7_200_000) >= 2);
    assert_eq!(report(&grid)["paused"], false);
}

/// Past `MAX_HISTORY_CANDLES` the filter would always start unwarmed, with nothing saying why.
#[test]
fn the_warm_up_never_needs_more_bars_than_the_host_gives() {
    let filter = |slow_secs: u64| {
        let tf = serde_json::json!({ "slow_secs": slow_secs, "pause_divergence_bps": "50" });
        GridStrategy::new(with(test_config(), serde_json::json!({ "trend_filter": tf })))
    };
    let need = filter(u64::MAX).history().unwrap();
    let max = bb_strategy_sdk::MAX_HISTORY_CANDLES;
    assert_eq!((need.want, need.need), (max, max));
    assert_eq!(filter(3_600).history().unwrap().need, 180, "three slow windows of 1m bars");
}
