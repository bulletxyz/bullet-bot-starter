//! The port's `decide` against the built-in's: every view bullet-bots' funding-arb tests gave its
//! `decide` (`parity/fixtures/funding-arb/`), replayed here; the targets must be the same.

use std::path::Path;

use bb_strategy_funding_arb_wasm::decide::{MarketView, decide};
use serde_json::Value;

fn port(view: &Value) -> Result<Value, String> {
    let view: MarketView = serde_json::from_value(view.clone()).map_err(|e| e.to_string())?;
    serde_json::to_value(decide(&view)).map_err(|e| e.to_string())
}

fn dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/funding-arb")
}

#[test]
fn the_port_decides_every_recorded_view_as_the_built_in_did() {
    let reports =
        bb_strategy_parity::funding_arb::replay_all(&dir(), port).unwrap_or_else(|e| panic!("{e}"));
    let views: usize = reports.iter().map(|r| r.views).sum();
    let targets: usize = reports.iter().map(|r| r.targets).sum();
    for r in &reports {
        eprintln!("{}: {} views, {} targets", r.test, r.views, r.targets);
    }
    assert_eq!(reports.len(), 15, "fixtures replayed");
    assert!(views >= 500 && targets >= views, "{views} views, {targets} targets");
}

/// The replay is not vacuous: a `decide` that sizes one dollar differently fails it.
#[test]
fn a_port_that_sizes_differently_fails_the_replay() {
    let skewed = |view: &Value| {
        let mut out = port(view)?;
        for t in out.as_array_mut().into_iter().flatten() {
            if t["notional_usd"] != "0" {
                t["notional_usd"] = "1".into();
            }
        }
        Ok(out)
    };
    let e = bb_strategy_parity::funding_arb::replay_all(&dir(), skewed).unwrap_err();
    assert!(e.contains("the built-in returned"), "{e}");
}
