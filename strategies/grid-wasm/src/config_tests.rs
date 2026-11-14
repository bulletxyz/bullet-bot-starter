use super::*;

fn parse(s: &str) -> Result<GridConfig, serde_json::Error> {
    serde_json::from_str(s)
}

const RANGE: &str = r#""lower_price": "70", "upper_price": "80", "num_levels": 6"#;

#[test]
fn defaults_are_the_built_ins() {
    let c = parse(&format!("{{{RANGE}}}")).unwrap();
    assert_eq!(c.exchange, "bullet");
    assert_eq!(c.amount_usd, DEFAULT_AMOUNT_USD);
    assert_eq!(c.direction, Direction::Neutral);
    assert_eq!(c.order_type, OrderType::PostOnly);
    assert_eq!(c.stop_beyond, DEFAULT_STOP_BEYOND);
    assert!(c.fees.is_none() && c.trend_filter.is_none() && c.max_position.is_none());
    c.check().unwrap();
}

#[test]
fn without_a_range_and_levels_it_cannot_run_as_a_plugin() {
    let e = parse("{}").unwrap().check().unwrap_err();
    assert!(e.0.contains("lower_price, upper_price and num_levels"), "{e}");
}

#[test]
fn more_levels_than_one_call_takes_are_refused() {
    let c = parse(r#"{"lower_price": "70", "upper_price": "80", "num_levels": 64}"#).unwrap();
    assert!(c.check().unwrap_err().0.contains("at most 63"));
}

#[test]
fn validate_refuses_bad_values() {
    for bad in [
        r#"{"exchange": "binance"}"#,
        r#"{"amount_usd": "0"}"#,
        r#"{"num_levels": 1}"#,
        r#"{"lower_price": "80", "upper_price": "70"}"#,
        r#"{"lower_price": "0", "upper_price": "70"}"#,
        r#"{"order_size": "0"}"#,
        r#"{"max_position": "0"}"#,
        r#"{"stop_beyond": "0"}"#,
        r#"{"lower_price": "70", "upper_price": "80", "anchor_price": "90"}"#,
        r#"{"symbol": " "}"#,
    ] {
        assert!(parse(bad).map_or(true, |c| c.validate().is_err()), "{bad}");
    }
    assert!(parse(r#"{"nope": 1}"#).is_err(), "unknown keys are refused");
}

/// What the bot hands a plugin: its section without the symbol and its own limits, over the
/// declared defaults in the JSON the bot writes (a decimal as text, an int as a number, a choice
/// as its name).
#[test]
fn the_declared_defaults_as_the_bot_writes_them_parse() {
    let bot = r#"{"amount_usd": "100", "direction": "neutral", "order_type": "post_only",
        "stop_beyond": "0.5", "lower_price": "70", "upper_price": "80", "num_levels": 6}"#;
    parse(bot).unwrap().check().unwrap();
}

#[test]
fn no_declared_setting_is_one_of_the_bots_limits_or_an_override_without_a_default() {
    for s in settings() {
        assert!(
            !["symbol", "max_position", "anchor_price", "order_size", "exchange", "range"]
                .contains(&s.path.as_str())
                && !s.path.starts_with("fees.")
                && !s.path.starts_with("trend_filter."),
            "{}",
            s.path
        );
    }
    let required: Vec<_> = settings().into_iter().filter(|s| s.required).map(|s| s.path).collect();
    assert_eq!(required, ["lower_price", "upper_price", "num_levels"]);
}
