//! The port against the built-in: every call bullet-bots' grid tests made to it under the
//! bot's real host (`parity/fixtures/grid/`), replayed here; the intents must be the same.

use std::path::Path;

use bb_strategy_grid_wasm::GridStrategy;
use bb_strategy_sdk::FromSettings;

#[test]
fn the_port_answers_every_recorded_call_as_the_built_in_did() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/grid");
    let reports = bb_strategy_parity::replay_all(&dir, GridStrategy::from_settings)
        .unwrap_or_else(|e| panic!("{e}"));
    let replayed: Vec<_> = reports.iter().filter(|r| r.skipped.is_none()).collect();
    let intents: usize = replayed.iter().map(|r| r.intents).sum();
    for r in &reports {
        eprintln!(
            "{}: {} calls, {} intents, {} headlines{}",
            r.test,
            r.calls,
            r.intents,
            r.headlines,
            r.skipped.as_deref().map(|s| format!(" (skipped: {s})")).unwrap_or_default()
        );
    }
    let skipped = reports.iter().filter(|r| r.skipped.is_some()).count();
    assert_eq!((replayed.len(), skipped), (41, 6), "fixtures replayed and skipped");
    assert!(intents > 0, "no intents compared");
}

/// The replay is not vacuous: a grid whose range sits one dollar higher fails it.
#[test]
fn a_port_that_lays_its_levels_differently_fails_the_replay() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/grid");
    let shifted = |settings: &str| {
        let mut json: serde_json::Value = serde_json::from_str(settings).unwrap();
        if let Some(lower) = json["lower_price"].as_str() {
            let lower: rust_decimal::Decimal = lower.parse().unwrap();
            json["lower_price"] = (lower + rust_decimal::Decimal::ONE).to_string().into();
        }
        GridStrategy::from_settings(&json.to_string())
    };
    let e = bb_strategy_parity::replay_all(&dir, shifted).unwrap_err();
    assert!(e.contains("the built-in returned"), "{e}");
}
