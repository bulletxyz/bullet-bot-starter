//! The grid's range and step history through the port's list of history needs (A3), in place of
//! the build's `with_history`.

use std::collections::BTreeMap;

use bb_strategy_sdk::{
    Candle, CandleInterval, HistoryNeed, Intent, MarketFacts, OrderBook, StrategyAdapter, View,
    Warmup,
};
use rust_decimal::Decimal;

use crate::adapter::GridStrategy;
use crate::config::GridConfig;
use crate::plan::History;

const NOON: u64 = 1_794_398_400_000;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

/// A derived grid: the range, the levels and the size come from the history.
fn derived() -> GridConfig {
    serde_json::from_value(serde_json::json!({ "symbol": "SOL-USD" })).unwrap()
}

fn candle(c: &str, spread: &str) -> Candle {
    let (c, spread) = (d(c), d(spread));
    Candle {
        open_ms: 0,
        open: c,
        high: c + spread,
        low: c - spread,
        close: c,
        volume: Decimal::ZERO,
    }
}

/// 31 daily closes alternating 80 and 83.2 and 15 hourly bars spanning 0.5 (as `plan_tests`).
fn history() -> History {
    let daily = (0..31).map(|i| candle(if i % 2 == 0 { "80" } else { "83.2" }, "1")).collect();
    let hourly = (0..15).map(|_| candle("80", "0.25")).collect();
    History { daily, hourly }
}

fn loaded(candles: Vec<Candle>) -> Warmup {
    Warmup::Loaded { candles, source: "Binance".into(), now_ms: NOON }
}

fn book() -> OrderBook {
    let bids = BTreeMap::from([(d("79.99"), d("10"))]);
    let asks = BTreeMap::from([(d("80.01"), d("10"))]);
    OrderBook { bids, asks, last_update_id: 0 }
}

fn view(book: &OrderBook) -> View<'_> {
    View {
        now_ms: NOON,
        symbol: "SOL-USD",
        book: Some(book),
        position: Decimal::ZERO,
        avg_entry: None,
        realized_pnl: Decimal::ZERO,
        open_orders: Some(&[]),
        positions: Some(&[]),
        reconnected: false,
        venue_halt: None,
        path_stop: None,
        read_started_ms: None,
        limits: None,
        market: MarketFacts {
            tick_size: Some(d("0.001")),
            step_size: Some(d("0.01")),
            ..MarketFacts::default()
        },
        legs: &[],
    }
}

/// The orders a start places, without their client ids (each grid issues its own).
fn orders(intents: &[Intent]) -> Vec<String> {
    intents
        .iter()
        .map(|i| match i {
            Intent::Place(o) => {
                format!("{:?} {} {} {:?}", o.side, o.price, o.quantity, o.order_type)
            }
            other => format!("{other:?}"),
        })
        .collect()
}

#[test]
fn a_derived_grid_asks_for_its_daily_and_hourly_candles() {
    let need = |interval, n| HistoryNeed { interval, want: n, need: n };
    // Normal: a 30 day lookback, so 31 closes; the ATR over 14 hours, so 15 bars.
    let grid = GridStrategy::new(derived());
    assert_eq!(grid.histories(), [need(CandleInterval::D1, 31), need(CandleInterval::H1, 15)]);
    // The build's history, or a range and levels given in full: nothing to read.
    assert!(GridStrategy::new(derived()).with_history(history()).histories().is_empty());
    let given: GridConfig = serde_json::from_value(serde_json::json!({
        "symbol": "SOL-USD", "lower_price": "70", "upper_price": "90", "num_levels": 5
    }))
    .unwrap();
    assert!(GridStrategy::new(given).histories().is_empty());
}

#[test]
fn candles_the_host_loads_lay_the_grid_the_builds_history_did() {
    let b = book();
    let mut built = GridStrategy::new(derived()).with_history(history());
    let want = built.start(&view(&b)).unwrap();
    let mut loaded_grid = GridStrategy::new(derived());
    for need in loaded_grid.histories() {
        let h = history();
        let bars = if need.interval == CandleInterval::D1 { h.daily } else { h.hourly };
        loaded_grid.warm_up_for(need, loaded(bars));
    }
    let got = loaded_grid.start(&view(&b)).unwrap();
    assert!(!want.is_empty());
    assert_eq!(orders(&got), orders(&want));
}

/// No history is no grid: the start is refused with the reason the host gave.
#[test]
fn a_grid_whose_candles_could_not_be_loaded_refuses_to_start_and_says_why() {
    let mut grid = GridStrategy::new(derived());
    for need in grid.histories() {
        let warmup = if need.interval == CandleInterval::D1 {
            Warmup::Unavailable("Binance refused (HTTP 451)".into())
        } else {
            loaded(history().hourly)
        };
        grid.warm_up_for(need, warmup);
    }
    let b = book();
    let e = grid.start(&view(&b)).unwrap_err();
    assert!(e.0.contains("can't set its range for SOL-USD: Binance refused (HTTP 451)"), "{e}");
}
