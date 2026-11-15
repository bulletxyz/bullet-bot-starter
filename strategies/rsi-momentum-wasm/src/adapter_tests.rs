//! What the port does differently from the built-in (the rest is the parity fixtures' job).

use bb_strategy_sdk::testing::{Market, SimHost, d};
use bb_strategy_sdk::{Event, FieldValue, FromSettings, Intent, StrategyAdapter, StrategyReport};
use bb_strategy_support::is_own;

use super::*;

const SYM: &str = "BTC-USD";
const MIN: u64 = 60_000;
/// A UTC day boundary.
const T0: u64 = 20_000 * 1_440 * MIN;

fn strategy(extra: &str) -> RsiMomentum {
    let json = format!(
        r#"{{"interval": "1m", "rsi_period": 2, "notional_usd": "100", "cooldown_candles": 0,
            "stop_loss_pct": "0.2"{extra}}}"#
    );
    RsiMomentum::from_settings(&json).unwrap()
}

fn market() -> Market {
    let mut m = Market::new(SYM, "99.95", "100.05", T0);
    m.market.step_size = Some(d("0.001"));
    m.positions = Some(Vec::new());
    m
}

/// Marks closing one 1m candle each at `closes`, then a tick with a fresh flat venue read.
fn candles(sim: &mut SimHost<RsiMomentum>, closes: &[&str]) -> Vec<Intent> {
    let mut sent = Vec::new();
    for (i, close) in closes.iter().enumerate() {
        sim.market.now_ms = T0 + (i as u64 + 1) * MIN + 1;
        sim.market.positions = Some(Vec::new());
        let mid = d(close);
        sim.market.book = Some(bb_strategy_sdk::testing::book(
            &(mid - d("0.05")).to_string(),
            &(mid + d("0.05")).to_string(),
            "1000",
        ));
        sent.extend(sim.send(&Event::Tick));
        sent.extend(sim.send(&Event::Mark(d(close))));
    }
    sent
}

#[test]
fn the_start_sends_nothing_and_says_it_warms_up_live() {
    let mut sim = SimHost::new(strategy(""), market());
    assert!(sim.start().unwrap().is_empty(), "no alert to resolve: the world has none");
    let report = sim.strategy.report();
    assert_eq!(report.history.as_deref(), Some(LIVE_WARMUP));
    assert!(report.headline().starts_with("BTC-USD: warming up"), "{}", report.headline());
}

#[test]
fn a_cross_enters_with_the_bots_own_client_id() {
    let mut sim = SimHost::new(strategy(""), market());
    sim.start().unwrap();
    // Period-2 closes: the RSI crosses above 60 at the close of the sixth candle.
    let sent = candles(&mut sim, &["100", "101", "102", "103", "99", "104", "104"]);
    let entry = sent.iter().find_map(|i| if let Intent::Place(o) = i { Some(o) } else { None });
    let entry = entry.expect("an entry");
    assert!(is_own(entry.client_id.as_deref().unwrap()), "{entry:?}");
    assert!(!entry.reduce_only);
}

#[test]
fn settings_naming_another_market_are_refused_at_start() {
    let mut s = strategy(r#", "symbol": "ETH-USD""#);
    assert!(s.start(&market().view()).is_err());
    assert!(strategy(r#", "symbol": "BTC-USD""#).start(&market().view()).is_ok());
}

#[test]
fn the_report_has_the_built_ins_fields() {
    let mut sim = SimHost::new(strategy(""), market());
    sim.start().unwrap();
    let fields = sim.strategy.report().fields();
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    for name in ["symbol", "interval", "closes_to_warm", "cooldown_candles_left", "close_failing"] {
        assert!(names.contains(&name), "{name}: {names:?}");
    }
    let failing = fields.iter().find(|f| f.name == "close_failing").unwrap();
    assert_eq!(failing.value, FieldValue::Flag(false));
}
