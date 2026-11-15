use super::*;

fn parse(s: &str) -> Result<AvellanedaStoikovSettings, serde_json::Error> {
    serde_json::from_str(s)
}

const MIN: &str = r#""order_size": "1", "max_position": "5", "gamma": "1", "kappa": "1.5""#;

fn ok(extra: &str) -> AvellanedaStoikovSettings {
    parse(&format!("{{{MIN}{extra}}}")).unwrap()
}

#[test]
fn defaults_are_the_built_ins() {
    let c = ok("");
    assert_eq!(c.order_horizon_secs, 60);
    assert_eq!((c.vol_window_secs, c.order_refresh_secs), (600, 10));
    assert_eq!(c.min_half_spread_bps, Decimal::from(5));
    assert_eq!(c.max_half_spread_bps, Decimal::from(500));
    assert_eq!((c.order_levels, c.order_level_spread_bps), (3, Decimal::from(10)));
    assert_eq!(c.order_type, OrderType::PostOnly);
    assert_eq!(c.amend_threshold_bps, Decimal::new(5, 1));
    assert_eq!((c.min_refresh_interval_ms, c.reconcile_interval_secs), (100, 10));
    assert_eq!((c.reference_stale_secs, c.reference_market.as_str()), (10, "perp"));
    assert!(!c.uses_reference());
    c.validate().unwrap();
}

#[test]
fn the_required_settings_are_required() {
    for missing in ["order_size", "max_position", "gamma", "kappa"] {
        let mut json: serde_json::Value = serde_json::from_str(&format!("{{{MIN}}}")).unwrap();
        json.as_object_mut().unwrap().remove(missing);
        assert!(serde_json::from_value::<AvellanedaStoikovSettings>(json).is_err(), "{missing}");
    }
}

#[test]
fn validate_refuses_what_the_built_in_refuses() {
    for bad in [
        r#", "exchange": "binance""#,
        r#", "symbol": " ""#,
        r#", "order_size": "0""#,
        r#", "max_position": "0.5""#,
        r#", "gamma": "0""#,
        r#", "kappa": "-1""#,
        r#", "order_horizon_secs": 0"#,
        r#", "vol_window_secs": 0"#,
        r#", "min_half_spread_bps": "0""#,
        r#", "max_half_spread_bps": "4""#,
        r#", "order_levels": 0"#,
        r#", "order_level_spread_bps": "-1""#,
        r#", "order_level_amount_step": "-0.1""#,
        r#", "amend_threshold_bps": "-1""#,
        r#", "amend_threshold_step_bps": "-1""#,
        r#", "reference_exchange": "binance""#,
        r#", "reference_stale_secs": 0"#,
        r#", "fees": {"maker_bps": "-1"}"#,
        r#", "fees": {"maker_bps": "1", "min_spread_fee_multiplier": "0.5"}"#,
    ] {
        let refused = parse(&format!("{{{MIN}{bad}}}")).map_or(true, |c| c.validate().is_err());
        assert!(refused, "{bad}");
    }
}

#[test]
fn a_bot_cannot_take_more_levels_than_its_intent_cap_allows() {
    ok(&format!(r#", "order_levels": {MAX_LEVELS}"#)).validate().unwrap();
    let e = ok(&format!(r#", "order_levels": {}"#, MAX_LEVELS + 1)).validate().unwrap_err();
    assert!(e.0.contains("at most"), "{e}");
}

#[test]
fn unknown_settings_are_refused() {
    assert!(parse(&format!(r#"{{{MIN}, "gama": "1"}}"#)).is_err());
}

#[test]
fn the_reference_mode_needs_its_symbol() {
    let c = ok(r#", "reference_exchange": "binance", "reference_symbol": "btcusdt""#);
    assert!(c.uses_reference());
    c.validate().unwrap();
}

#[test]
fn the_fee_floor_refuses_a_spread_the_fees_eat() {
    // 2 x 5 bp is below 3 x 2 x 4 bp.
    let c = ok(r#", "fees": {"maker_bps": "4"}"#);
    c.validate().unwrap();
    let e = c.check_fee_floor().unwrap_err();
    assert!(e.0.contains("Raise min_half_spread_bps"), "{e}");
    ok(r#", "fees": {"maker_bps": "1"}"#).check_fee_floor().unwrap();
    ok("").check_fee_floor().unwrap();
}

#[test]
fn the_settings_are_unique_and_inside_their_ranges() {
    let all = settings();
    let mut paths: Vec<&str> = all.iter().map(|s| s.path.as_str()).collect();
    paths.sort_unstable();
    let n = paths.len();
    paths.dedup();
    assert_eq!(paths.len(), n, "a path twice");
    for s in &all {
        if let (Some(g), Some(h)) = (s.guarded.min, s.hard.min) {
            assert!(h <= g, "{}: hard covers guarded", s.path);
        }
        if let (Some(g), Some(h)) = (s.guarded.max, s.hard.max) {
            assert!(h >= g, "{}: hard covers guarded", s.path);
        }
    }
    let required: Vec<&str> = all.iter().filter(|s| s.required).map(|s| s.path.as_str()).collect();
    assert_eq!(required, ["order_size", "max_position", "gamma", "kappa"]);
}
