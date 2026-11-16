use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bb_strategy_sdk::{
    Event, Intent, LimitsInForce, MarketFacts, NewOrder, Order, OrderBook, OrderResult,
    OrderStatus, OrderType, Side, StrategyAdapter, Trade, View,
};
use rust_decimal::Decimal;

use crate::GridConfig;
use crate::adapter::GridStrategy;
use crate::plan::HardLimits;

const NOON: u64 = 1_794_398_400_000;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

/// 74..78 in 5 levels around 75.5 (halfway, so none is empty): buys 74, 75; sells 76, 77, 78.
/// Neutral: the hard stops are half the width (2) past each edge, 72 and 80.
fn test_config() -> GridConfig {
    serde_json::from_value(serde_json::json!({
        "symbol": "BTC-USD", "lower_price": "74", "upper_price": "78", "num_levels": 5,
        "anchor_price": "75.5", "order_size": "1", "order_type": "limit", "max_position": "10",
        "amount_usd": "1000"
    }))
    .unwrap()
}

#[expect(clippy::needless_pass_by_value, reason = "called with temporaries")]
fn with(cfg: GridConfig, v: serde_json::Value) -> GridConfig {
    let mut c = serde_json::json!({
        "symbol": cfg.symbol, "lower_price": cfg.lower_price, "upper_price": cfg.upper_price,
        "num_levels": cfg.num_levels, "anchor_price": cfg.anchor_price,
        "order_size": cfg.order_size, "order_type": "limit", "max_position": cfg.max_position,
        "amount_usd": cfg.amount_usd
    });
    for (k, x) in v.as_object().unwrap() {
        c[k] = x.clone();
    }
    serde_json::from_value(c).unwrap()
}

fn book(bid: &str, ask: &str) -> OrderBook {
    let bids = BTreeMap::from([(d(bid), d("1"))]);
    let asks = BTreeMap::from([(d(ask), d("1"))]);
    OrderBook { bids, asks, last_update_id: 0 }
}

fn mid_book() -> OrderBook {
    book("75.4", "75.6")
}

fn view<'a>(book: Option<&'a OrderBook>, open: &'a [Order], now_ms: u64) -> View<'a> {
    at(book, open, now_ms, Decimal::ZERO)
}

fn at<'a>(book: Option<&'a OrderBook>, open: &'a [Order], now_ms: u64, pos: Decimal) -> View<'a> {
    View {
        now_ms,
        symbol: "BTC-USD",
        book,
        position: pos,
        avg_entry: None,
        realized_pnl: Decimal::ZERO,
        open_orders: Some(open),
        positions: Some(&[]),
        reconnected: false,
        venue_halt: None,
        path_stop: None,
        read_started_ms: None,
        limits: None,
        market: MarketFacts::default(),
        legs: &[],
    }
}

fn trade(client_id: Option<&str>, side: Side, price: &str) -> Trade {
    Trade {
        exchange: "bullet".into(),
        symbol: "BTC-USD".into(),
        order_id: "42".into(),
        trade_id: None,
        client_id: client_id.map(Into::into),
        side,
        price: d(price),
        quantity: d("1"),
        timestamp: None,
    }
}

fn placements(intents: &[Intent]) -> Vec<&NewOrder> {
    intents.iter().filter_map(|i| if let Intent::Place(o) = i { Some(o) } else { None }).collect()
}

fn cancels(intents: &[Intent]) -> Vec<String> {
    intents
        .iter()
        .filter_map(|i| if let Intent::Cancel(c) = i { c.client_id.clone() } else { None })
        .collect()
}

fn acked(order: &NewOrder) -> Event {
    let result = OrderResult {
        order_id: Some(format!("oid-{}", order.price)),
        client_id: order.client_id.clone(),
        success: true,
        error: None,
    };
    Event::Outcome { intent: Intent::Place(order.clone()), result }
}

fn cancelled(client_id: &str) -> Order {
    Order {
        id: String::new(),
        client_id: Some(client_id.into()),
        symbol: "BTC-USD".into(),
        side: Side::Buy,
        order_type: OrderType::Limit,
        price: d("74"),
        quantity: d("1"),
        filled_quantity: Decimal::ZERO,
        status: OrderStatus::Cancelled,
    }
}

/// `(state, has a client id)` of each level, as the strategy reports them.
fn levels(strategy: &GridStrategy) -> Vec<(String, bool)> {
    strategy
        .report()
        .levels
        .iter()
        .map(|l| (format!("{:?}", l.state), l.client_id.is_some()))
        .collect()
}

/// The report as MCP and the web UI read it.
fn report(strategy: &GridStrategy) -> serde_json::Value {
    serde_json::to_value(strategy.report()).unwrap()
}

/// A started grid whose placements the venue acked.
fn running(cfg: GridConfig) -> (GridStrategy, Vec<NewOrder>) {
    running_on(GridStrategy::new(cfg), &mid_book())
}

fn running_on(mut grid: GridStrategy, b: &OrderBook) -> (GridStrategy, Vec<NewOrder>) {
    let intents = grid.start(&view(Some(b), &[], NOON)).unwrap();
    let orders: Vec<NewOrder> = placements(&intents).into_iter().cloned().collect();
    for order in &orders {
        grid.on_event(&acked(order), &view(Some(b), &[], NOON));
    }
    (grid, orders)
}

/// The venue lists each of these orders as resting.
fn resting(orders: &[NewOrder]) -> Vec<Order> {
    orders
        .iter()
        .map(|o| Order { status: OrderStatus::Open, ..cancelled(o.client_id.as_deref().unwrap()) })
        .collect()
}

/// The orders `intents` placed, each answered unknown (the venue left the call unanswered).
fn unknown_all(grid: &mut GridStrategy, intents: &[Intent], b: &OrderBook) {
    for order in placements(intents) {
        let unknown = Event::Unknown { intent: Intent::Place(order.clone()), error: "t".into() };
        grid.on_event(&unknown, &view(Some(b), &[], NOON));
    }
}

fn prices_of(intents: &[Intent], side: Side) -> Vec<String> {
    placements(intents).iter().filter(|o| o.side == side).map(|o| o.price.to_string()).collect()
}

#[test]
fn unmatched_trade_is_ignored() {
    let (mut grid, _) = running(test_config());
    let event = Event::Fill(trade(Some("unknown"), Side::Buy, "75"));
    assert!(grid.on_event(&event, &view(Some(&mid_book()), &[], NOON)).is_empty());
}

#[test]
fn lifecycle_cancelled_flips_active_to_pending() {
    let (mut grid, orders) = running(test_config());
    assert!(levels(&grid).iter().all(|(state, _)| state == "Active"));
    let cid = orders[2].client_id.clone().unwrap();
    let out = grid.on_event(&Event::Order(cancelled(&cid)), &view(Some(&mid_book()), &[], NOON));
    assert!(out.is_empty(), "the tick re-places, not the event: {out:?}");
    assert_eq!(levels(&grid)[2], ("Pending".to_string(), false));
    assert_eq!(levels(&grid)[1].0, "Active");
}

#[test]
fn lifecycle_cancelled_noop_when_already_pending() {
    // The placement went out but its answer is not back: the level is Pending with its client id.
    let mut grid = GridStrategy::new(test_config());
    let b = mid_book();
    let intents = grid.start(&view(Some(&b), &[], NOON)).unwrap();
    let cid = placements(&intents)[1].client_id.clone().unwrap();
    grid.on_event(&Event::Order(cancelled(&cid)), &view(Some(&b), &[], NOON));
    assert_eq!(levels(&grid)[1], ("Pending".to_string(), true));
}

/// A post-only level whose price would cross is repriced to the touch on its own side (still a
/// maker order), not dropped.
#[test]
fn a_post_only_level_that_would_cross_goes_at_the_touch() {
    // Anchor 76 empties 76: buys 74, 75, sells 77, 78. The ask is 74.9, so the buy at 75 would
    // cross; it goes at the bid, 74.8.
    let cfg =
        with(test_config(), serde_json::json!({ "order_type": "post_only", "anchor_price": "76" }));
    let mut grid = GridStrategy::new(cfg);
    let b = book("74.8", "74.9");
    let intents = grid.start(&view(Some(&b), &[], NOON)).unwrap();
    let sent: Vec<(Side, String)> =
        placements(&intents).iter().map(|o| (o.side, o.price.to_string())).collect();
    let want = [(Side::Buy, "74"), (Side::Buy, "74.8"), (Side::Sell, "77"), (Side::Sell, "78")];
    assert_eq!(sent, want.map(|(s, p)| (s, p.to_string())));
    assert!(placements(&intents).iter().all(|o| o.order_type == OrderType::PostOnly));
}

/// The level nearest the mid gets no order: neutral starts with buys below it and sells above.
#[test]
fn the_level_nearest_the_mid_is_left_empty() {
    let cfg = with(test_config(), serde_json::json!({ "anchor_price": null }));
    let mut grid = GridStrategy::new(cfg);
    let b = book("75.9", "76.1");
    let intents = grid.start(&view(Some(&b), &[], NOON)).unwrap();
    let prices: Vec<String> = placements(&intents).iter().map(|o| o.price.to_string()).collect();
    assert_eq!(prices, ["74", "75", "77", "78"], "76 is the mid's level");
}

#[test]
fn a_tick_replaces_a_level_the_venue_no_longer_shows() {
    let (mut grid, _) = running(test_config());
    let out = grid.on_event(&Event::Tick, &view(Some(&mid_book()), &[], NOON + 3_000));
    // A read past the settle margin lists nothing, so all five levels come back and are placed
    // again.
    assert_eq!(placements(&out).len(), 5);
}

#[test]
fn stop_cancels_everything() {
    let mut grid = GridStrategy::new(test_config());
    assert!(matches!(grid.stop(&view(None, &[], NOON)).as_slice(), [Intent::CancelAll]));
}

#[test]
fn start_refuses_a_config_it_cannot_run() {
    let cfg = with(test_config(), serde_json::json!({ "num_levels": 1 }));
    assert!(GridStrategy::new(cfg).start(&view(Some(&mid_book()), &[], NOON)).is_err());
}

#[test]
fn start_needs_a_book() {
    assert!(GridStrategy::new(test_config()).start(&view(None, &[], NOON)).is_err());
}

/// The grid starts from no position: one already on the account would sit outside its worst case.
#[test]
fn start_refuses_a_position_already_held() {
    let b = mid_book();
    let e = GridStrategy::new(test_config()).start(&at(Some(&b), &[], NOON, d("0.5"))).unwrap_err();
    assert!(e.0.contains("already holds 0.5"), "{e}");
}

// --- Rule 1: the worst case fits the dollars and the hard limits. ---

#[test]
fn start_refuses_a_worst_case_past_the_hard_limits() {
    // Sells 76, 77, 78 of 1 each: a short worst case of 3 coins, $231.
    let b = mid_book();
    let tight = HardLimits { max_position: Some(d("2")), max_notional_usd: None };
    let e = GridStrategy::new(test_config()).with_limits(tight).start(&view(Some(&b), &[], NOON));
    assert!(e.unwrap_err().0.contains("position limit 2"));
    let poor = HardLimits { max_position: None, max_notional_usd: Some(d("230")) };
    let e = GridStrategy::new(test_config()).with_limits(poor).start(&view(Some(&b), &[], NOON));
    assert!(e.unwrap_err().0.contains("notional limit $230"));
    let mine = with(test_config(), serde_json::json!({ "amount_usd": "200" }));
    assert!(GridStrategy::new(mine).start(&view(Some(&b), &[], NOON)).is_err());
    let fits = HardLimits { max_position: Some(d("3")), max_notional_usd: Some(d("231")) };
    GridStrategy::new(test_config()).with_limits(fits).start(&view(Some(&b), &[], NOON)).unwrap();
}

/// The limits the host holds the path to, shown in the view, refuse the same plans as the build's;
/// they win over the build's when both are given.
#[test]
fn the_limits_in_the_view_refuse_a_worst_case_past_them_as_the_builds_do() {
    let b = mid_book();
    let start = |limits: LimitsInForce, built: Option<HardLimits>| {
        let mut grid = GridStrategy::new(test_config());
        if let Some(built) = built {
            grid = grid.with_limits(built);
        }
        grid.start(&View { limits: Some(limits), ..view(Some(&b), &[], NOON) })
    };
    let tight = LimitsInForce::new(d("2"), d("50"), None);
    assert!(start(tight.clone(), None).unwrap_err().0.contains("position limit 2"));
    let poor = LimitsInForce {
        max_notional_usd: Some(d("230")),
        ..LimitsInForce::new(d("10"), d("50"), None)
    };
    assert!(start(poor, None).unwrap_err().0.contains("notional limit $230"));
    let fits = LimitsInForce {
        max_notional_usd: Some(d("231")),
        ..LimitsInForce::new(d("3"), d("50"), None)
    };
    let loose = HardLimits { max_position: Some(d("100")), max_notional_usd: None };
    assert!(start(tight, Some(loose)).is_err());
    start(fits, Some(HardLimits { max_position: Some(d("1")), max_notional_usd: None })).unwrap();
}

// --- Rule 5: P&L shown honestly. ---

#[test]
fn the_report_shows_realized_and_unrealized_of_the_whole_position() {
    let (mut grid, _) = running(test_config());
    let b = book("74.9", "75.1");
    let v = View {
        avg_entry: Some(d("76")),
        realized_pnl: d("1.5"),
        ..at(Some(&b), &[], NOON, d("2"))
    };
    grid.on_event(&Event::Book, &v);
    let pnl = &report(&grid)["pnl"];
    // Long 2 from 76 at a mid of 75: -2 open, +1.5 closed.
    assert_eq!((pnl["realized"].as_str(), pnl["unrealized"].as_str()), (Some("1.5"), Some("-2")));
    assert_eq!(pnl["total"].as_str(), Some("-0.5"));
}

/// The config check refuses the multiplier, and the floor's own product is checked too.
#[test]
fn a_fee_floor_past_decimal_max_refuses_the_start_not_a_panic() {
    let fees = serde_json::json!({
        "maker_bps": "1000", "min_spacing_fee_multiplier": "100000000000000000000000000"
    });
    let mut grid = GridStrategy::new(with(test_config(), serde_json::json!({ "fees": fees })));
    let e = grid.start(&view(Some(&mid_book()), &[], NOON)).unwrap_err();
    assert!(e.0.contains("min_spacing_fee_multiplier"), "{e:?}");
    let market = crate::plan::Market { mid: d("75.5"), tick: None, lot: None };
    let plan = crate::plan::resolve(&grid.config, market, None, HardLimits::default()).unwrap();
    assert!(grid.check_fee_floor(&plan).is_err());
}

#[path = "adapter_direction_tests.rs"]
mod direction_tests;
#[path = "adapter_keep_tests.rs"]
mod keep_tests;
#[path = "adapter_orders_tests.rs"]
mod orders_tests;
#[path = "adapter_pauses_tests.rs"]
mod pauses_tests;
#[path = "adapter_stops_tests.rs"]
mod stops_tests;
