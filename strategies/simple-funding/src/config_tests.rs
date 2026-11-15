use bb_strategy_sdk::param_spec::{BoundsMode, ParamKind};

use super::*;

fn parse(s: &str) -> Result<SimpleFundingConfig, toml::de::Error> {
    toml::from_str(s)
}

#[test]
fn every_declaration_is_well_formed() {
    for spec in PARAM_SPECS {
        spec.check_declaration().unwrap_or_else(|e| panic!("{}: {e}", spec.name));
    }
}

#[test]
fn empty_section_has_the_spec_defaults() {
    let c = parse("").unwrap();
    assert_eq!(c.notional_per_symbol, Decimal::from(50));
    assert_eq!((c.top_k, c.max_positions), (2, 4));
    assert_eq!(c.max_net_pct, hundredths(15));
    assert_eq!(c.max_oi_pct, hundredths(5));
    assert_eq!(c.max_leverage, Decimal::ONE);
    assert_eq!((c.signal_window_hours, c.expected_hold_hours), (24, 72));
    assert_eq!(c.slippage_bps, Decimal::from(10));
    assert_eq!(c.min_depth_usd, Decimal::from(5000));
    assert_eq!(c.daily_loss_usd, None, "no stop is configured, a paper run assumes one");
    assert_eq!(c.loss_stop_usd(), DEFAULT_DAILY_LOSS_USD);
    assert_eq!(c.stale_secs, 30);
    c.validate().unwrap();
}

#[test]
fn defaults_are_conservative() {
    let c = parse("").unwrap();
    assert_eq!(c.notional_per_symbol, Decimal::from(50));
    assert_eq!((c.top_k, c.max_positions), (2, 4));
    assert_eq!(c.max_leverage, Decimal::ONE);
    assert_eq!(c.max_net_pct, hundredths(15));
    assert!(!c.adopt_existing_positions);
    // The whole default book is at most 4 x 50 USD, and gross stays inside equity.
    assert!(c.notional_per_symbol * Decimal::from(c.max_positions) <= Decimal::from(200));
}

#[test]
fn a_live_config_without_an_explicit_loss_stop_is_refused() {
    let err = parse("").unwrap().check_live().unwrap_err();
    assert!(err.contains("daily_loss_usd"), "{err}");
    // Paper may omit it: it runs on the paper default.
    parse("").unwrap().validate().unwrap();
}

#[test]
fn a_live_config_with_an_unsafe_loss_stop_is_refused() {
    for bad in [
        "daily_loss_usd = \"0\"",
        "daily_loss_usd = \"-5\"",
        // More than the whole 4 x 50 book: it could never trip before ruin.
        "daily_loss_usd = \"201\"",
        "daily_loss_usd = \"1000000\"",
    ] {
        let live = parse(bad).unwrap();
        assert!(live.validate().and_then(|()| live.check_live()).is_err(), "live: {bad}");
    }
    for bad in ["daily_loss_usd = \"0\"", "daily_loss_usd = \"-5\""] {
        assert!(parse(bad).unwrap().validate().is_err(), "paper too: {bad}");
    }
    parse("daily_loss_usd = \"25\"").unwrap().check_live().unwrap();
    parse("daily_loss_usd = \"200\"").unwrap().check_live().unwrap();
}

#[test]
fn unknown_keys_are_refused() {
    assert!(parse("hedge_exchange = \"hyperliquid\"").is_err());
}

/// The declared defaults are the config's, and each declared hard bound is the one
/// `validate` enforces.
#[test]
fn param_specs_match_the_config() {
    let c = parse("").unwrap();
    let value = |name: &str| match name {
        "notional_per_symbol" => c.notional_per_symbol,
        "top_k" => Decimal::from(c.top_k),
        "max_positions" => Decimal::from(c.max_positions),
        "max_net_pct" => c.max_net_pct,
        "max_oi_pct" => c.max_oi_pct,
        "max_leverage" => c.max_leverage,
        "signal_window_hours" => Decimal::from(c.signal_window_hours),
        "expected_hold_hours" => Decimal::from(c.expected_hold_hours),
        "slippage_bps" => c.slippage_bps,
        "min_depth_usd" => c.min_depth_usd,
        "daily_loss_usd" => c.loss_stop_usd(),
        "stale_secs" => Decimal::from(c.stale_secs),
        "adopt_existing_positions" => Decimal::from(u8::from(c.adopt_existing_positions)),
        other => panic!("{other} has no config field"),
    };
    for spec in PARAM_SPECS {
        assert_eq!(spec.default, value(spec.name), "{} default", spec.name);
        let range = spec.range(BoundsMode::Extended);
        let Some(max) = range.max.filter(|_| !matches!(spec.kind, ParamKind::Bool)) else {
            continue;
        };
        let over = match spec.kind {
            ParamKind::Int => format!("{} = {}", spec.name, max + Decimal::ONE),
            _ => format!("{} = \"{}\"", spec.name, max + Decimal::ONE),
        };
        assert!(parse(&over).unwrap().validate().is_err(), "{over} must be refused");
    }
}

#[test]
fn limits_loosen_upward_and_depth_downward() {
    let spec = |n: &str| PARAM_SPECS.iter().find(|s| s.name == n).unwrap();
    for name in ["notional_per_symbol", "top_k", "max_positions", "max_net_pct", "max_leverage"] {
        assert_eq!(spec(name).tier, Tier::Limit, "{name}");
        assert_eq!(spec(name).loosening, Loosening::Up, "{name}");
    }
    assert_eq!(spec("min_depth_usd").loosening, Loosening::Down);
}

#[test]
fn validate_refuses_bad_values() {
    for bad in [
        "notional_per_symbol = \"0\"",
        "top_k = 0",
        "top_k = 26",
        "max_positions = 0",
        "max_net_pct = \"1.5\"",
        "max_net_pct = \"-0.1\"",
        "max_oi_pct = \"0\"",
        "max_leverage = \"6\"",
        "signal_window_hours = 0",
        "expected_hold_hours = 721",
        "slippage_bps = \"0\"",
        "slippage_bps = \"101\"",
        "min_depth_usd = \"-1\"",
        "daily_loss_usd = \"0\"",
        "stale_secs = 4",
        "only = [\"A\"]\nexclude = [\"A\"]",
    ] {
        assert!(parse(bad).unwrap().validate().is_err(), "{bad}");
    }
}

#[test]
fn the_universe_lists_are_limits_so_widening_one_needs_raise_limits() {
    let settings = settings();
    for name in ["only", "exclude"] {
        let spec = settings.iter().find(|s| s.path == name).unwrap().spec;
        assert_eq!((spec.tier, spec.loosening), (Tier::Limit, Loosening::Up), "{name}");
    }
}
