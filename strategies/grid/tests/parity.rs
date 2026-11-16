//! A regression pin: every call the grid's own tests made to it under the bot's real host
//! (`parity/fixtures/grid/`), replayed on this crate; the intents must be what they were.

use std::path::Path;

use bb_strategy_grid::GridStrategy;
use bb_strategy_sdk::{FromSettings, StrategyError};

/// The bot's build: the strategy from its settings, through the port.
fn build(settings: &str) -> Result<GridStrategy, StrategyError> {
    GridStrategy::from_settings(settings)
}

#[test]
fn the_grid_answers_every_recorded_call_as_it_did_under_the_host() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/grid");
    let reports = bb_strategy_parity::replay_all(&dir, build).unwrap_or_else(|e| panic!("{e}"));
    let intents: usize = reports.iter().map(|r| r.intents).sum();
    eprintln!("{} fixtures replayed, {intents} intents compared", reports.len());
    assert!(intents > 0, "no intents compared");
}

/// The replay is not vacuous: a grid whose range sits one dollar higher fails it.
#[test]
fn a_grid_that_lays_its_levels_differently_fails_the_replay() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/grid");
    let shifted = |settings: &str| {
        let mut json: serde_json::Value = serde_json::from_str(settings).unwrap();
        if let Some(lower) = json["lower_price"].as_str() {
            let lower: rust_decimal::Decimal = lower.parse().unwrap();
            json["lower_price"] = (lower + rust_decimal::Decimal::ONE).to_string().into();
        }
        build(&json.to_string())
    };
    let e = bb_strategy_parity::replay_all(&dir, shifted).unwrap_err();
    assert!(e.contains("the built-in returned"), "{e}");
}
