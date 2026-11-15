use bb_strategy_sdk::param_spec::{BoundsMode, ParamKind};

use super::*;

fn parse(s: &str) -> Result<RsiMomentumConfig, toml::de::Error> {
    toml::from_str(&format!("symbol = \"BTC-USD\"\nmax_position = \"0.01\"\n{s}"))
}

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

#[test]
fn every_declaration_is_well_formed() {
    for s in settings() {
        s.spec.check_declaration().unwrap_or_else(|e| panic!("{}: {e}", s.path));
    }
}

#[test]
fn defaults_are_a_conservative_long_only_paper_run() {
    let c = parse("").unwrap();
    assert!(!c.allow_short, "long only by default");
    assert_eq!(c.interval, CandleInterval::M15);
    assert_eq!(c.rsi_period, 14);
    assert_eq!((c.long_above, c.short_below, c.exit_band), (d("60"), d("40"), d("50")));
    assert_eq!(c.notional_usd, d("50"));
    assert_eq!(c.stop_loss_pct, d("0.02"));
    assert_eq!(c.daily_loss_usd, None);
    assert_eq!(c.loss_stop_usd(), DEFAULT_DAILY_LOSS_USD);
    assert_eq!((c.cooldown_candles, c.stale_secs), (2, 30));
    c.validate().unwrap();
}

#[test]
fn symbol_and_max_position_have_no_default() {
    assert!(toml::from_str::<RsiMomentumConfig>("max_position = \"1\"").is_err());
    assert!(toml::from_str::<RsiMomentumConfig>("symbol = \"BTC-USD\"").is_err());
}

#[test]
fn a_live_config_needs_an_explicit_bounded_loss_stop() {
    let err = parse("").unwrap().check_live().unwrap_err();
    assert!(err.contains("daily_loss_usd"), "{err}");
    for bad in ["0", "-1", "50.01"] {
        let live = parse(&format!("daily_loss_usd = \"{bad}\"")).unwrap();
        assert!(live.validate().and_then(|()| live.check_live()).is_err(), "{bad}");
    }
    parse("daily_loss_usd = \"5\"").unwrap().check_live().unwrap();
    parse("daily_loss_usd = \"50\"").unwrap().check_live().unwrap();
}

#[test]
fn validate_refuses_bad_values() {
    for bad in [
        "rsi_period = 1",
        "rsi_period = 101",
        "long_above = \"101\"",
        "short_below = \"-1\"",
        "short_below = \"60\"",
        "exit_band = \"65\"",
        "exit_band = \"35\"",
        "notional_usd = \"0\"",
        "max_position = \"0\"",
        "stop_loss_pct = \"0\"",
        "stop_loss_pct = \"0.51\"",
        "daily_loss_usd = \"0\"",
        "cooldown_candles = 101",
        "slippage_bps = \"0\"",
        "slippage_bps = \"101\"",
        "stale_secs = 4",
        "interval = \"2m\"",
    ] {
        let parsed = toml::from_str::<RsiMomentumConfig>(&format!(
            "symbol = \"BTC-USD\"\n{}\n{bad}",
            if bad.starts_with("max_position") { "" } else { "max_position = \"0.01\"" }
        ));
        assert!(parsed.map_or(true, |c| c.validate().is_err()), "{bad}");
    }
    assert!(
        toml::from_str::<RsiMomentumConfig>("symbol = \"\"\nmax_position = \"1\"")
            .unwrap()
            .validate()
            .is_err()
    );
}

/// Any other name is refused, naming the ones it takes.
#[test]
fn the_interval_is_read_by_name() {
    for name in ["1m", "5m", "15m", "30m", "1h", "4h", "1d"] {
        assert_eq!(parse(&format!("interval = \"{name}\"")).unwrap().interval.name(), name);
    }
    let err = parse("interval = \"2m\"").unwrap_err().to_string();
    assert!(
        err.contains(
            "unknown variant `2m`, expected one of `1m`, `5m`, `15m`, `30m`, `1h`, `4h`, `1d`"
        ),
        "{err}"
    );
}

#[test]
fn unknown_keys_are_refused() {
    assert!(parse("hedge_exchange = \"hyperliquid\"").is_err());
}

/// The declared defaults are the config's, and each declared hard bound is the one `validate`
/// enforces.
#[test]
fn param_specs_match_the_config() {
    let c = parse("").unwrap();
    let value = |name: &str| match name {
        "interval" => Decimal::from(
            CandleInterval::NAMES.iter().position(|n| *n == c.interval.name()).unwrap(),
        ),
        "rsi_period" => Decimal::from(c.rsi_period),
        "long_above" => c.long_above,
        "short_below" => c.short_below,
        "exit_band" => c.exit_band,
        "allow_short" => Decimal::from(u8::from(c.allow_short)),
        "notional_usd" => c.notional_usd,
        "stop_loss_pct" => c.stop_loss_pct,
        "daily_loss_usd" => c.loss_stop_usd(),
        "cooldown_candles" => Decimal::from(c.cooldown_candles),
        "slippage_bps" => c.slippage_bps,
        "stale_secs" => Decimal::from(c.stale_secs),
        other => panic!("{other} has no config field"),
    };
    for spec in PARAM_SPECS {
        assert_eq!(spec.default, value(spec.name), "{} default", spec.name);
        let range = spec.range(BoundsMode::Extended);
        let Some(max) =
            range.max.filter(|_| !matches!(spec.kind, ParamKind::Bool | ParamKind::Enum { .. }))
        else {
            continue;
        };
        let over = match spec.kind {
            ParamKind::Int => format!("{} = {}", spec.name, max + Decimal::ONE),
            _ => format!("{} = \"{}\"", spec.name, max + Decimal::ONE),
        };
        assert!(parse(&over).map_or(true, |c| c.validate().is_err()), "{over} must be refused");
    }
}
