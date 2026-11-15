use super::*;

fn parse(s: &str) -> Result<RsiMomentumSettings, serde_json::Error> {
    serde_json::from_str(s)
}

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

#[test]
fn defaults_are_a_conservative_long_only_run() {
    let c = parse("{}").unwrap();
    assert!(!c.allow_short, "long only by default");
    assert_eq!(c.interval, Interval::M15);
    assert_eq!(c.rsi_period, 14);
    assert_eq!((c.long_above, c.short_below, c.exit_band), (d("60"), d("40"), d("50")));
    assert_eq!(c.notional_usd, d("50"));
    assert_eq!(c.stop_loss_pct, d("0.02"));
    assert_eq!((c.cooldown_candles, c.stale_secs), (2, 30));
    assert_eq!((c.max_position, c.daily_loss_usd), (None, None), "the bot's limits");
    c.validate().unwrap();
}

#[test]
fn validate_refuses_bad_values() {
    for bad in [
        r#"{"rsi_period": 1}"#,
        r#"{"rsi_period": 101}"#,
        r#"{"long_above": "101"}"#,
        r#"{"short_below": "-1"}"#,
        r#"{"short_below": "60"}"#,
        r#"{"exit_band": "65"}"#,
        r#"{"exit_band": "35"}"#,
        r#"{"notional_usd": "0"}"#,
        r#"{"max_position": "0"}"#,
        r#"{"stop_loss_pct": "0"}"#,
        r#"{"stop_loss_pct": "0.51"}"#,
        r#"{"daily_loss_usd": "0"}"#,
        r#"{"cooldown_candles": 101}"#,
        r#"{"slippage_bps": "0"}"#,
        r#"{"slippage_bps": "101"}"#,
        r#"{"stale_secs": 4}"#,
        r#"{"interval": "2m"}"#,
        r#"{"symbol": " "}"#,
    ] {
        assert!(parse(bad).map_or(true, |c| c.validate().is_err()), "{bad}");
    }
}

#[test]
fn unknown_keys_are_refused() {
    assert!(parse(r#"{"hedge_exchange": "hyperliquid"}"#).is_err());
}

/// What the bot hands a plugin: its section without the symbol and its own limits, over the
/// declared defaults (each in the JSON shape the bot writes: a decimal as text, an int as a
/// number, a flag as a bool, a choice as its name).
#[test]
fn the_declared_defaults_as_the_bot_writes_them_parse_to_the_defaults() {
    let bot = r#"{"interval": "15m", "rsi_period": 14, "long_above": "60", "short_below": "40",
        "exit_band": "50", "allow_short": false, "notional_usd": "50", "stop_loss_pct": "0.02",
        "cooldown_candles": 2, "slippage_bps": "10"}"#;
    assert_eq!(parse(bot).unwrap(), parse("{}").unwrap());
}

#[test]
fn every_declared_setting_is_a_field_and_none_is_one_of_the_bots_limits() {
    let fields = serde_json::to_value(parse("{}").unwrap()).unwrap();
    for s in settings() {
        assert!(fields.get(&s.path).is_some(), "{} is no field", s.path);
        assert!(s.hard.min.is_none_or(|m| s.guarded.min.is_some_and(|g| g >= m)), "{}", s.path);
    }
    for host in ["symbol", "max_position", "daily_loss_usd", "stale_secs", "dry_run"] {
        assert!(settings().iter().all(|s| s.path != host), "{host} is the bot's");
    }
}

#[test]
fn risk_settings_are_limits_that_loosen_upward() {
    let all = settings();
    let get = |name: &str| all.iter().find(|s| s.path == name).unwrap();
    for name in ["notional_usd", "stop_loss_pct", "allow_short", "slippage_bps"] {
        assert_eq!((get(name).tier, get(name).loosening), (Tier::Limit, Loosening::Up), "{name}");
    }
    assert_eq!(
        get("cooldown_candles").loosening,
        Loosening::Down,
        "a shorter cooldown trades more"
    );
}
