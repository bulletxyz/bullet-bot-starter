use super::*;

fn fill(paper: bool, taker: bool) -> FillRecord {
    FillRecord {
        id: 1,
        ts_ms: 0,
        side: Side::Buy,
        price: Decimal::ONE,
        quantity: Decimal::ONE,
        mid_at_fill: Decimal::ONE,
        spread_captured_bps: 0.0,
        markout_5s_bps: None,
        markout_60s_bps: None,
        paper,
        taker,
        version: None,
        config_version: None,
        trade_id: None,
    }
}

/// An unknown fee is zero.
#[test]
fn a_fill_pays_its_sides_fee_and_a_live_one_the_higher() {
    let fees = Fees { maker_bps: Some(Decimal::ONE), taker_bps: Some(Decimal::TWO) };
    assert_eq!(fees.of(&fill(true, false)), Decimal::ONE, "paper maker");
    assert_eq!(fees.of(&fill(true, true)), Decimal::TWO, "paper taker");
    assert_eq!(fees.of(&fill(false, false)), Decimal::TWO, "live maker");
    assert_eq!(fees.of(&fill(false, true)), Decimal::TWO, "live taker");

    let maker_only = Fees { maker_bps: Some(Decimal::ONE), taker_bps: None };
    assert_eq!(maker_only.of(&fill(true, true)), Decimal::ZERO, "paper taker, no taker fee");
    assert_eq!(maker_only.of(&fill(false, false)), Decimal::ONE, "live, only the maker fee");

    let kind = |paper, taker| FillKind { paper, taker };
    assert_eq!(fees.of_kind(kind(true, false)), Decimal::ONE, "paper maker, by kind");
    assert_eq!(fees.of_kind(kind(true, true)), Decimal::TWO, "paper taker, by kind");
    assert_eq!(fees.of_kind(kind(false, false)), Decimal::TWO, "live, by kind");
    assert_eq!(maker_only.of_kind(kind(true, true)), Decimal::ZERO, "no taker fee, by kind");
}
