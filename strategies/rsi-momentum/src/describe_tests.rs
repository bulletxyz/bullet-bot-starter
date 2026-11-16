use bb_strategy_sdk::{Describe, FromSettings};

use crate::RsiMomentum;

#[test]
fn rsi_momentum_is_named_for_its_folder_and_lists_every_setting() {
    assert_eq!(RsiMomentum::NAME, "rsi-momentum");
    assert_eq!(RsiMomentum::VERSION, env!("CARGO_PKG_VERSION"));
    let d = RsiMomentum::describe();
    let paths: Vec<_> = d.settings.iter().map(|s| s.path.clone()).collect();
    let want: Vec<_> = crate::settings().into_iter().map(|s| s.path).collect();
    assert_eq!(paths, want);
    assert!(d.legs.is_empty());
}

#[test]
fn rsi_momentum_builds_from_its_settings_and_refuses_what_it_cannot_run() {
    let ok = r#"{"symbol": "BTC-USD", "max_position": "0.01", "interval": "15m"}"#;
    assert!(RsiMomentum::from_settings(ok).is_ok());
    let unknown =
        RsiMomentum::from_settings(r#"{"symbol": "BTC-USD", "max_position": "1", "x": 1}"#);
    assert!(unknown.is_err_and(|e| e.0.contains('x')));
    // Parsed, but refused by its own check.
    let crossed = r#"{"symbol": "BTC-USD", "max_position": "1", "short_below": "70"}"#;
    assert!(RsiMomentum::from_settings(crossed).is_err());
}
