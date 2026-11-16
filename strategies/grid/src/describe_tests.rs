use bb_strategy_sdk::{Describe, FromSettings};

use crate::GridStrategy;

#[test]
fn the_grid_is_named_for_its_folder_and_lists_every_setting() {
    assert_eq!(GridStrategy::NAME, "grid");
    assert_eq!(GridStrategy::VERSION, env!("CARGO_PKG_VERSION"));
    let d = GridStrategy::describe();
    let paths: Vec<_> = d.settings.iter().map(|s| s.path.clone()).collect();
    let want: Vec<_> = crate::config::settings().into_iter().map(|s| s.path).collect();
    assert_eq!(paths, want);
    assert!(d.legs.is_empty());
}

#[test]
fn the_grid_builds_from_its_settings_and_refuses_what_it_cannot_run() {
    assert!(GridStrategy::from_settings(r#"{"symbol": "SOL-USD"}"#).is_ok());
    let unknown = GridStrategy::from_settings(r#"{"symbol": "SOL-USD", "spread": 1}"#);
    assert!(unknown.is_err_and(|e| e.0.contains("spread")));
    // Parsed, but refused by its own check.
    assert!(GridStrategy::from_settings(r#"{"symbol": "SOL-USD", "amount_usd": "0"}"#).is_err());
}
