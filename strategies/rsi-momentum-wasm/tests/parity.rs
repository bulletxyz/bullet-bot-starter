//! The port against the built-in: every call bullet-bots' rsi-momentum tests made to it under the
//! bot's real host (`parity/fixtures/rsi-momentum/`), replayed here; the intents must be the same.

use std::path::Path;

use bb_strategy_rsi_momentum_wasm::RsiMomentum;
use bb_strategy_sdk::FromSettings;

#[test]
fn the_port_answers_every_recorded_call_as_the_built_in_did() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/rsi-momentum");
    let reports = bb_strategy_parity::replay_all(&dir, RsiMomentum::from_settings)
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
    assert_eq!((replayed.len(), skipped), (29, 3), "fixtures replayed and skipped");
    assert!(intents > 0, "no intents compared");
}

/// The replay is not vacuous: a port that crosses one bp further than the built-in fails it.
#[test]
fn a_port_that_prices_differently_fails_the_replay() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/rsi-momentum");
    let wider = |settings: &str| {
        let mut json: serde_json::Value = serde_json::from_str(settings).unwrap();
        let slip: rust_decimal::Decimal = json["slippage_bps"].as_str().unwrap().parse().unwrap();
        json["slippage_bps"] = (slip + rust_decimal::Decimal::ONE).to_string().into();
        RsiMomentum::from_settings(&json.to_string())
    };
    let e = bb_strategy_parity::replay_all(&dir, wider).unwrap_err();
    assert!(e.contains("the built-in returned"), "{e}");
}
