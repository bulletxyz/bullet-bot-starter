use serde_json::{Value, json};

use super::*;

fn parse(v: Value) -> Result<MarketMakerSettings, serde_json::Error> {
    serde_json::from_value(v)
}

fn base() -> Value {
    json!({
        "exchange": "bullet", "symbol": "BTC-USD", "bid_spread_bps": "10", "ask_spread_bps": "10",
        "order_size": "0.001", "max_position": "0.005",
        "fees": {"maker_bps": "1", "min_spread_fee_multiplier": "3"},
    })
}

fn with(changes: &Value) -> MarketMakerSettings {
    let mut v = base();
    for (k, val) in changes.as_object().unwrap() {
        v[k] = val.clone();
    }
    parse(v).unwrap()
}

#[test]
fn base_config_is_valid() {
    with(&json!({})).validate().unwrap();
}

#[test]
fn max_position_must_cover_order_size() {
    assert!(with(&json!({"max_position": "0.0005"})).validate().is_err());
}

#[test]
fn fee_floor_rejects_too_tight_spread() {
    assert!(with(&json!({"bid_spread_bps": "1", "ask_spread_bps": "1"})).validate().is_err());
}

#[test]
fn skew_limits_and_fair_value_fields_are_range_checked() {
    let bad = [
        json!({"skew_bps": "-1"}),
        json!({"skew_bps": "101"}),
        json!({"daily_loss_usd": "0"}),
        json!({"daily_loss_usd": "-1"}),
        json!({"stale_secs": 0}),
        json!({"max_orders_per_min": 0}),
        json!({"max_notional_usd": "0"}),
        json!({"basis_halflife_secs": 0}),
        json!({"reference_stale_secs": 0}),
        json!({"max_divergence_bps": "0"}),
        json!({"basis_halflife_secs": 9}),
        json!({"basis_halflife_secs": 3601}),
        json!({"reference_stale_secs": 11}),
        json!({"max_divergence_bps": "4"}),
        json!({"max_divergence_bps": "501"}),
        json!({"refresh_secs": 0}),
        json!({"refresh_threshold_bps": "-1"}),
    ];
    for c in bad {
        assert!(with(&c).validate().is_err(), "{c}");
    }
    with(&json!({"skew_bps": "100", "daily_loss_usd": "1", "stale_secs": 1})).validate().unwrap();
}

/// A v51 section still loads: the keys that are now the instance's or the host's are accepted and
/// ignored, and an unknown key is still refused.
#[test]
fn a_v51_section_loads_with_the_keys_that_are_no_longer_its_own() {
    let v51 = with(&json!({
        "dry_run": true, "state_dir": "/data", "max_inflight": 2,
        "guards": {"fast_move_bps": 20, "rejects_to_trip": 3},
        "max_position_fallback": false,
    }));
    v51.validate().unwrap();
    let mut v = base();
    v["spread"] = json!(1);
    assert!(parse(v).unwrap_err().to_string().contains("spread"));
}

#[test]
fn binance_symbol_defaults_to_base_usdt() {
    assert_eq!(with(&json!({})).binance_symbol(), "BTCUSDT");
    assert_eq!(with(&json!({"binance_symbol": "ethusdt"})).binance_symbol(), "ETHUSDT");
    assert_eq!(with(&json!({"binance_symbol": " BTCUSDT "})).binance_symbol(), "BTCUSDT");
}

#[test]
fn blank_binance_symbol_is_rejected() {
    for blank in ["", "  "] {
        let err = with(&json!({"binance_symbol": blank})).validate().unwrap_err();
        assert!(err.contains("binance_symbol"), "{err}");
    }
}

#[test]
fn defaults_price_off_binance() {
    let c = parse(json!({
        "symbol": "BTC-USD", "bid_spread_bps": 2, "ask_spread_bps": 2,
        "order_size": 1, "max_position": 1,
    }))
    .unwrap();
    assert_eq!(c.fair_value, FairValue::Binance);
    assert_eq!((c.basis_halflife_secs, c.reference_stale_secs), (300, 5));
    assert_eq!(c.max_divergence_bps, Decimal::from(20));
    assert!(c.skew_bps.is_zero());
    assert_eq!((c.refresh_secs, c.refresh_threshold_bps), (5, Decimal::ONE));
    assert_eq!(c.order_type, OrderType::PostOnly);
    assert_eq!(c.max_orders_per_min, DEFAULT_MAX_ORDERS_PER_MIN);
    assert_eq!(c.max_notional_usd, None);
    assert!(c.extra.is_empty());
}

#[test]
fn extra_table_sets_starting_values() {
    let c = with(&json!({"extra": {"vol_widen_bps": "7.5", "vol_widen_enabled": true}}));
    let params = MmParams::from_settings(&c);
    assert_eq!(params.extra["vol_widen_bps"], Decimal::new(75, 1));
    assert!(params.flag("vol_widen_enabled"), "a switch may be written as a flag");
}

/// Spreads and fees no one bounded must not overflow the fee-floor arithmetic: a huge value is an
/// error, not a panic.
#[test]
fn huge_spreads_and_fees_are_an_error_not_a_panic() {
    let max = Decimal::MAX.to_string();
    let c = with(&json!({"bid_spread_bps": max, "ask_spread_bps": max}));
    assert!(c.validate().is_err_and(|e| e.contains("too large")));
    let c = with(&json!({"fees": {"maker_bps": max, "min_spread_fee_multiplier": max}}));
    assert!(c.validate().is_err_and(|e| e.contains("too large")));
}

/// Every key of the section is a described setting, but the ones that are not the strategy's any
/// more; the declarations are consistent.
#[test]
fn every_setting_is_described_and_its_declaration_is_consistent() {
    let all = settings();
    let paths: Vec<&str> = all.iter().map(|s| s.path.as_str()).collect();
    let schema = serde_json::to_value(schemars::schema_for!(MarketMakerSettings)).unwrap();
    let keys: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
    for key in keys {
        let described = paths.iter().any(|p| p.split('.').next() == Some(key.as_str()));
        assert!(described, "{key} has no setting");
    }
    for s in &all {
        assert_eq!(s.spec.check_declaration(), Ok(()), "{}", s.path);
    }
    assert!(all.iter().any(|s| s.path == "symbol" && s.required));
    assert!(all.iter().any(|s| s.path == "extra.vol_widen_bps"));
    assert!(!paths.iter().any(|p| ["dry_run", "max_inflight", "state_dir"].contains(p)));
    assert!(!paths.iter().any(|p| p.starts_with("guards.")), "the guards are the host's");
}

/// The setting defaults make a valid section (the required ones at their example values).
#[test]
fn the_defaults_make_a_valid_section() {
    let mut section = serde_json::Map::new();
    for s in settings() {
        let value = s.default_json();
        match s.path.split_once('.') {
            Some((table, key)) => {
                let t = section.entry(table).or_insert_with(|| json!({}));
                t[key] = value;
            }
            None => {
                section.insert(s.path.clone(), value);
            }
        }
    }
    parse(Value::Object(section)).unwrap().validate().unwrap();
}
