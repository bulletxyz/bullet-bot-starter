//! The port against the built-in: every call bullet-bots' avellaneda-stoikov made in scripted
//! scenarios (`parity/fixtures/avellaneda-stoikov/`, recorded from the real actor), replayed here;
//! the intents must be the same.

use std::path::Path;

use bb_strategy_avellaneda_stoikov_wasm::AvellanedaStoikov;
use bb_strategy_sdk::FromSettings;

fn dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/avellaneda-stoikov")
}

#[test]
fn the_port_answers_every_recorded_call_as_the_built_in_did() {
    let reports =
        bb_strategy_parity::avellaneda::replay_all(&dir(), AvellanedaStoikov::from_settings)
            .unwrap_or_else(|e| panic!("{e}"));
    let intents: usize = reports.iter().map(|r| r.intents).sum();
    let calls: usize = reports.iter().map(|r| r.calls).sum();
    for r in &reports {
        eprintln!("{}: {} calls, {} intents", r.test, r.calls, r.intents);
    }
    assert_eq!(reports.len(), 22, "fixtures replayed");
    assert!(reports.iter().all(|r| r.skipped.is_none()), "none is skipped");
    assert!(calls > 150 && intents > 100, "{calls} calls, {intents} intents compared");
}

/// The replay is not vacuous: a port with one more bp of amend threshold, or a different gamma,
/// fails it.
#[test]
fn a_port_that_quotes_differently_fails_the_replay() {
    let tweak = |key: &'static str, value: &'static str| {
        move |settings: &str| {
            let mut json: serde_json::Value = serde_json::from_str(settings).unwrap();
            json[key] = value.into();
            AvellanedaStoikov::from_settings(&json.to_string())
        }
    };
    for (key, value) in [("gamma", "1.01"), ("amend_threshold_bps", "30"), ("kappa", "1.6")] {
        let e = bb_strategy_parity::avellaneda::replay_all(&dir(), tweak(key, value)).unwrap_err();
        assert!(e.contains("the built-in returned"), "{key}: {e}");
    }
}
