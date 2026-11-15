//! Settings, the descriptor and the start's refusals.

use bb_strategy_sdk::testing::d;
use bb_strategy_sdk::{Describe, FromSettings, StrategyAdapter};

use super::harness::{Sim, settings};
use crate::SimpleFunding;

#[test]
fn the_defaults_are_the_built_ins() {
    let s: crate::SimpleFundingSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(s.notional_per_symbol, d("50"));
    assert_eq!((s.top_k, s.max_positions), (2, 4));
    assert_eq!((s.max_net_pct, s.max_oi_pct, s.max_leverage), (d("0.15"), d("0.05"), d("1")));
    assert_eq!((s.signal_window_hours, s.expected_hold_hours), (24, 72));
    assert_eq!((s.slippage_bps, s.min_depth_usd, s.stale_secs), (d("10"), d("5000"), 30));
    assert!(!s.adopt_existing_positions);
    assert_eq!(s.loss_stop_usd(), d("20"));
}

#[test]
fn settings_the_built_in_refuses_are_refused() {
    for bad in [
        r#"{"top_k": 0}"#,
        r#"{"max_positions": 51}"#,
        r#"{"max_oi_pct": "0.6"}"#,
        r#"{"max_leverage": "6"}"#,
        r#"{"slippage_bps": "0"}"#,
        r#"{"stale_secs": 4}"#,
        r#"{"daily_loss_usd": "0"}"#,
        r#"{"exclude": ["A"], "only": ["A"]}"#,
        r#"{"unknown": 1}"#,
    ] {
        assert!(SimpleFunding::from_settings(bad).is_err(), "{bad}");
    }
}

#[test]
fn a_daily_loss_that_is_no_stop_is_refused() {
    // 4 positions x 50 USD = 200 USD of book: a 500 USD stop never fires.
    assert!(SimpleFunding::from_settings(r#"{"daily_loss_usd": "500"}"#).is_err());
    assert!(SimpleFunding::from_settings(r#"{"daily_loss_usd": "200"}"#).is_ok());
}

#[test]
fn it_declares_the_universe_leg_and_the_built_ins_settings() {
    let d = SimpleFunding::describe();
    assert_eq!(SimpleFunding::NAME, "simple-funding-wasm");
    assert_eq!(d.legs.len(), 1);
    assert_eq!((d.legs[0].id.as_str(), d.legs[0].venue.as_str()), ("universe", "bullet"));
    let names: Vec<_> = d.settings.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(
        names,
        [
            "notional_per_symbol",
            "top_k",
            "max_positions",
            "max_net_pct",
            "max_oi_pct",
            "max_leverage",
            "signal_window_hours",
            "expected_hold_hours",
            "slippage_bps",
            "min_depth_usd",
            "daily_loss_usd",
            "stale_secs",
            "adopt_existing_positions",
            "exclude",
            "only"
        ]
    );
}

#[test]
fn it_does_not_start_without_the_leg_or_without_a_market_it_can_size() {
    let mut sim = Sim::new(settings(""), &["S1"]);
    sim.market.legs.clear();
    assert!(sim.start().unwrap_err().0.contains("universe"));
    let mut sim = Sim::new(settings(""), &["S1"]);
    sim.market.legs[0].market.clear();
    assert!(sim.start().unwrap_err().0.contains("no tradable markets"));
}

#[test]
fn only_and_exclude_narrow_the_universe() {
    let mut sim = Sim::new(settings(r#""exclude": ["L1"]"#), &["S1", "L1", "S2"]);
    sim.start().unwrap();
    sim.tick();
    assert!(sim.orders().iter().all(|o| o.symbol != "L1"));
    assert_eq!(sim.strategy.report().markets, 2);
}
