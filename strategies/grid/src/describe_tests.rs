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

#[test]
fn the_grid_profile_is_the_bots_descriptor() {
    let p = GridStrategy::profile().unwrap();
    assert_eq!(p.about.title, "Grid");
    assert!(p.supports_paper && p.has_loss_stop);
    assert_eq!(p.settings, crate::config::settings());
    assert_eq!((p.exchanges[0].role, p.exchanges[0].setting), ("venue", Some("exchange")));
    let featured: Vec<_> = p.about.featured.iter().map(|f| f.path).collect();
    assert_eq!(featured, ["amount_usd", "direction", "range"]);
}

/// The grid is sized in dollars alone: its range and levels come from the market at start.
#[test]
fn the_grid_preset_is_a_neutral_normal_grid_of_the_amount() {
    let p = GridStrategy::profile().unwrap().presets;
    let [sol] = p.as_slice() else { panic!("one preset: {p:?}") };
    assert_eq!((sol.id, sol.symbol, sol.needs_market), ("sol-grid", Some("SOL-USD"), false));
    let s = (sol.settings)(rust_decimal::Decimal::from(250), None).unwrap();
    assert_eq!(s["amount_usd"], "250");
    assert_eq!((s["direction"].as_str(), s["range"].as_str()), ("neutral", "normal"));
    let worst = (sol.worst_case)(rust_decimal::Decimal::from(250));
    assert!(worst.contains("$250") && worst.contains("stop"), "{worst}");
}
