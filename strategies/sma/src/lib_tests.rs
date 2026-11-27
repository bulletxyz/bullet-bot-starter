//! The average warmed up on past candles, and live: through the SDK's simulated host.

use bb_strategy_sdk::testing::{Market, SimHost, book, closed_candles, d};
use bb_strategy_sdk::{
    CandleInterval, Event, FromSettings, StrategyAdapter, StrategyReport, Warmup,
};

use super::*;

/// 2026-11-08 12:00 UTC, 25 s into a 15m bar.
const NOW: u64 = 1_794_398_425_000;
const BAR: u64 = 900_000;

fn sma(settings: &str) -> Sma {
    Sma::from_settings(settings).unwrap()
}

fn host(period: u32) -> SimHost<Sma> {
    let strategy = sma(&format!(r#"{{"period": {period}}}"#));
    SimHost::new(strategy, Market::new("BTC-USD", "99", "101", NOW))
}

fn loaded(closes: &[&str]) -> Warmup {
    let candles = closed_candles(CandleInterval::M15, NOW, closes);
    Warmup::Loaded { candles, source: "Binance".into(), now_ms: NOW }
}

/// A tick at `now_ms` over a `bid` / `ask` book.
fn tick(host: &mut SimHost<Sma>, now_ms: u64, bid: &str, ask: &str) {
    host.market.now_ms = now_ms;
    host.market.set_book(Some(book(bid, ask, "1")));
    assert!(host.send(&Event::Tick).is_empty(), "it places nothing");
}

#[test]
fn it_asks_for_period_bars_of_its_interval() {
    let need = sma(r#"{"period": 30, "interval": "1h"}"#).history().unwrap();
    assert_eq!((need.interval, need.want, need.need), (CandleInterval::H1, 30, 30));
}

#[test]
fn warmed_up_on_past_candles_it_has_its_average_at_start() {
    let mut host = host(4);
    host.warm_up(loaded(&["90", "94", "98", "102"]));
    assert_eq!(host.start().unwrap(), [], "it places nothing");
    let report = host.strategy.report();
    assert_eq!(report.average, Some(d("96")));
    assert_eq!(report.bars, 4);
    assert_eq!(report.warmed, "4 past bars from Binance");
    assert_eq!(report.headline(), "SMA(4, 15m) 96: the price is above it");
}

#[test]
fn the_live_bar_meets_the_last_past_one_with_no_gap_and_none_counted_twice() {
    let mut host = host(3);
    host.warm_up(loaded(&["10", "20", "30"]));
    host.start().unwrap();
    // The same bar: nothing closes.
    tick(&mut host, NOW + 60_000, "49", "51");
    assert_eq!(host.strategy.average(), Some(d("20")));
    // The next bar: the live one closes at its last mid (50) and the oldest drops.
    tick(&mut host, NOW + BAR, "59", "61");
    assert_eq!(host.strategy.average(), Some(d("100") / d("3")));
    assert_eq!(host.strategy.bars(), 3);
}

#[test]
fn without_past_candles_it_warms_up_live() {
    let mut host = host(2);
    host.warm_up(Warmup::Unavailable("no source of past prices is set up".into()));
    host.start().unwrap();
    let report = host.strategy.report();
    assert_eq!(report.headline(), "SMA(2, 15m) warming up: 0 of 2 bars");
    assert!(report.warmed.contains("no source of past prices"), "{}", report.warmed);
    tick(&mut host, NOW + BAR, "9", "11");
    tick(&mut host, NOW + 2 * BAR, "19", "21");
    assert_eq!(host.strategy.average(), Some(d("55")), "(100 + 10) / 2");
}

#[test]
fn settings_it_cannot_run_with_are_refused() {
    for bad in [r#"{"period": 1}"#, r#"{"period": 501}"#, r#"{"period": 5, "interval": "2m"}"#] {
        assert!(Sma::from_settings(bad).is_err(), "{bad}");
    }
}
