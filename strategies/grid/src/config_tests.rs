use super::*;

fn parse(v: serde_json::Value) -> GridConfig {
    serde_json::from_value(v).unwrap()
}

/// The `exchange` setting is advertised, but the build always trades on Bullet: any other
/// value would be accepted and ignored, so it is refused here.
#[test]
fn an_exchange_other_than_bullet_is_refused_not_ignored() {
    let config = |exchange: Option<&str>| -> GridConfig {
        let mut v = serde_json::json!({
            "symbol": "BTC-USD", "lower_price": "70000", "upper_price": "100000",
            "num_levels": 5, "order_size": "0.001", "max_position": "0.01"
        });
        if let Some(e) = exchange {
            v["exchange"] = e.into();
        }
        parse(v)
    };
    assert!(config(None).validate().is_ok());
    assert!(config(Some("bullet")).validate().is_ok());
    let e = config(Some("hyperliquid")).validate().unwrap_err();
    assert!(e.contains("exchange") && e.contains("bullet"), "{e}");
}

/// The owner gives three things; the rest is derived. Neutral, normal and $100 when left out, and
/// no history needed only when the range and the levels are all given. The mode is the
/// document's (paper until the owner's signed go-live), never the section's.
#[test]
fn a_symbol_alone_is_a_neutral_normal_grid_derived_from_history() {
    let c = parse(serde_json::json!({ "symbol": "SOL-USD" }));
    c.validate().unwrap();
    assert_eq!((c.direction, c.range), (Direction::Neutral, RangePreset::Normal));
    assert_eq!(c.amount_usd, DEFAULT_AMOUNT_USD);
    assert_eq!(c.stop_beyond, Decimal::new(5, 1), "half a range width past the edge");
    assert!(c.needs_history());
    let set = parse(serde_json::json!({
        "symbol": "SOL-USD", "direction": "long", "range": "tight", "amount_usd": "250",
        "lower_price": "70", "upper_price": "90", "num_levels": 9
    }));
    assert_eq!((set.direction, set.range), (Direction::Long, RangePreset::Tight));
    assert!(!set.needs_history());
    assert!(
        serde_json::from_value::<GridConfig>(serde_json::json!({
            "symbol": "SOL-USD", "direction": "sideways"
        }))
        .is_err()
    );
}

#[test]
fn the_hard_stop_cannot_be_turned_off_and_the_dollars_must_be_positive() {
    let base = || serde_json::json!({ "symbol": "SOL-USD" });
    let mut v = base();
    v["stop_beyond"] = "0".into();
    assert!(parse(v).validate().unwrap_err().contains("stop_beyond"));
    let mut v = base();
    v["amount_usd"] = "0".into();
    assert!(parse(v).validate().unwrap_err().contains("amount_usd"));
}

#[test]
fn every_declaration_is_well_formed() {
    for s in settings() {
        s.spec.check_declaration().unwrap_or_else(|e| panic!("{}: {e}", s.path));
    }
}

/// A config file booted with `bb run --config` is checked only by `validate`.
#[test]
fn validate_refuses_what_the_declared_hard_ranges_refuse() {
    use bb_strategy_sdk::param_spec::ParamKind;
    let config = |path: &str, kind: ParamKind, value: Decimal| {
        let mut v = serde_json::json!({ "symbol": "SOL-USD" });
        let value = match kind {
            ParamKind::Int => serde_json::json!(value.to_string().parse::<u64>().ok()),
            _ => serde_json::json!(value.to_string()),
        };
        match path.split_once('.') {
            Some((table, name)) => {
                v[table] = serde_json::json!({ "maker_bps": "0" });
                v[table][name] = value;
            }
            None => v[path] = value,
        }
        serde_json::from_value::<GridConfig>(v)
    };
    let mut checked = 0;
    for s in settings() {
        let spec = s.spec;
        if !matches!(spec.kind, ParamKind::Int | ParamKind::Decimal) {
            continue;
        }
        let step = |bound: Decimal| match spec.kind {
            ParamKind::Int => Decimal::ONE,
            _ if bound.is_zero() => Decimal::ONE,
            _ => bound.abs() / Decimal::from(100),
        };
        let past =
            [spec.hard.max.map(|m| (m, m + step(m))), spec.hard.min.map(|m| (m, m - step(m)))];
        for (bound, beyond) in past.into_iter().flatten() {
            let at = config(&s.path, spec.kind, bound).unwrap();
            at.validate().unwrap_or_else(|e| panic!("{} = {bound} must be accepted: {e}", s.path));
            // Past the bound it may not even parse (a negative count): refused either way.
            if let Ok(c) = config(&s.path, spec.kind, beyond) {
                let e = c.validate().expect_err(&format!("{} = {beyond} must be refused", s.path));
                assert!(e.contains(spec.name), "{} = {beyond}: {e}", s.path);
            }
            checked += 1;
        }
    }
    assert!(checked >= 8, "num_levels, anchor_price, stop_beyond and both fees: {checked}");
}

#[test]
fn the_history_bars_are_each_windows_closes_with_the_one_before() {
    let range = |r: &str| parse(serde_json::json!({ "symbol": "BTC-USD", "range": r }));
    assert_eq!(range("tight").history_bars(), (8, 15));
    assert_eq!(range("normal").history_bars(), (31, 15));
    assert_eq!(range("wide").history_bars(), (91, 15));
}

/// A wider preset moves the hard stop further out, as a lower `lower_price` does; past `normal` only the owner
/// signs.
#[test]
fn a_wider_range_preset_is_a_loosening_the_owner_signs() {
    use bb_strategy_sdk::param_spec::BoundsMode;
    let spec = settings().into_iter().find(|s| s.path == "range").unwrap().spec;
    let (tight, normal, wide) = (whole(0), whole(1), whole(2));
    assert!(spec.loosens(tight, wide) && spec.loosens(normal, wide));
    assert!(!spec.loosens(wide, tight));
    assert!(spec.check(normal, BoundsMode::Guarded).is_ok());
    assert!(spec.check(wide, BoundsMode::Guarded).is_err(), "past the default: the owner's");
    assert!(spec.check(wide, BoundsMode::Extended).is_ok());
}
