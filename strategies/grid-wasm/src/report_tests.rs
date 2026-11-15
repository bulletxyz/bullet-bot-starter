use bb_strategy_sdk::FieldValue;

use super::*;

fn report() -> GridReport {
    GridReport {
        direction: Some("neutral".into()),
        range: "normal".into(),
        lower_price: Some(Decimal::from(70)),
        upper_price: Some(Decimal::from(80)),
        anchor: None,
        spacing: None,
        order_size: None,
        stop_below: None,
        stop_above: None,
        worst_case_usd: None,
        amount_usd: None,
        active_levels: 4,
        total_levels: 5,
        paused: true,
        stopped: None,
        cancel_failures: 0,
        pnl: GridPnl::default(),
        levels: Vec::new(),
    }
}

#[test]
fn the_headline_is_the_built_ins() {
    let r = report();
    assert_eq!(r.headline(), "4 of 5 levels resting between 70 and 80, its opening orders paused");
    let stopped = GridReport { stopped: Some("hard stop".into()), ..r };
    assert_eq!(stopped.headline(), "stopped: hard stop");
}

#[test]
fn the_fields_name_the_plan_and_the_pnl() {
    let fields = report().fields();
    let get = |n: &str| fields.iter().find(|f| f.name == n).map(|f| f.value.clone());
    assert_eq!(get("paused"), Some(FieldValue::Flag(true)));
    assert_eq!(get("lower_price"), Some(FieldValue::Number(Decimal::from(70))));
    assert!(get("pnl.total").is_some() && get("anchor").is_none());
}

/// A stop that quotes a long refusal still reports within the bot's limits (past them the plugin
/// faults, and a faulted grid never closes on the next tick).
#[test]
fn a_long_stop_reason_is_clipped_to_the_bots_limits() {
    let r = GridReport { stopped: Some("x".repeat(300)), ..report() };
    assert!(r.headline().chars().count() <= MAX_REPORT_TEXT);
    let fields = r.fields();
    let stopped = fields.iter().find(|f| f.name == "stopped").unwrap();
    let FieldValue::Text(text) = &stopped.value else { panic!("{stopped:?}") };
    assert!(text.chars().count() <= MAX_REPORT_TEXT);
}
