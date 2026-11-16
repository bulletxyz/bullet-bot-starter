use super::*;

/// The snapshot's JSON keeps its shape: a cap on the notional shows only when there is one.
#[test]
fn the_limits_in_force_show_a_notional_cap_only_when_there_is_one() {
    let limits = LimitsInForce::new(Decimal::ONE, Decimal::TEN, Some(30));
    let json = serde_json::to_value(&limits).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"max_position": "1", "daily_loss_usd": "10", "stale_secs": 30})
    );
    let back: LimitsInForce = serde_json::from_value(json).unwrap();
    assert_eq!(back, limits);
    let capped = LimitsInForce { max_notional_usd: Some(Decimal::ONE_HUNDRED), ..limits };
    assert_eq!(serde_json::to_value(&capped).unwrap()["max_notional_usd"], "100");
}
