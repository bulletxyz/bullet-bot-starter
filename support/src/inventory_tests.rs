use super::*;
use rust_decimal::prelude::FromPrimitive;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

#[test]
fn open_long_no_pnl() {
    let mut inv = InventoryTracker::new();
    let pnl = inv.record_fill(Side::Buy, d("100"), d("1")).unwrap();
    assert_eq!(pnl, Decimal::ZERO);
    assert_eq!(inv.net_position, d("1"));
    assert_eq!(inv.avg_entry_price, d("100"));
}

#[test]
fn close_long_at_profit() {
    let mut inv = InventoryTracker::new();
    inv.record_fill(Side::Buy, d("100"), d("1"));
    let pnl = inv.record_fill(Side::Sell, d("110"), d("1")).unwrap();
    assert_eq!(pnl, d("10"));
    assert!(inv.is_flat());
    assert_eq!(inv.realized_pnl, d("10"));
    assert_eq!(inv.avg_entry_price, Decimal::ZERO);
}

#[test]
fn scale_in_weighted_average() {
    let mut inv = InventoryTracker::new();
    inv.record_fill(Side::Buy, d("100"), d("1"));
    inv.record_fill(Side::Buy, d("110"), d("1"));
    // avg = (100*1 + 110*1) / 2 = 105
    assert_eq!(inv.avg_entry_price, d("105"));
    assert_eq!(inv.net_position, d("2"));
}

#[test]
fn partial_close_keeps_residual() {
    let mut inv = InventoryTracker::new();
    inv.record_fill(Side::Buy, d("100"), d("2"));
    let pnl = inv.record_fill(Side::Sell, d("110"), d("1")).unwrap();
    assert_eq!(pnl, d("10"));
    assert_eq!(inv.net_position, d("1"));
    // Avg entry unchanged by partial close.
    assert_eq!(inv.avg_entry_price, d("100"));
}

#[test]
fn flip_splits_into_close_then_open() {
    let mut inv = InventoryTracker::new();
    inv.record_fill(Side::Buy, d("100"), d("1"));
    // Sell 2 at 110 → closes long with +10, opens short 1 @ 110.
    let pnl = inv.record_fill(Side::Sell, d("110"), d("2")).unwrap();
    assert_eq!(pnl, d("10"));
    assert_eq!(inv.net_position, d("-1"));
    assert_eq!(inv.avg_entry_price, d("110"));
}

#[test]
fn short_round_trip() {
    let mut inv = InventoryTracker::new();
    inv.record_fill(Side::Sell, d("110"), d("1"));
    let pnl = inv.record_fill(Side::Buy, d("100"), d("1")).unwrap();
    assert_eq!(pnl, d("10"));
    assert!(inv.is_flat());
}

#[test]
fn sanity_on_f64_roundtrip() {
    // Regression: avg calc must not blow up on non-round decimals.
    let mut inv = InventoryTracker::new();
    inv.record_fill(Side::Buy, Decimal::from_f64(100.5).unwrap(), d("0.5"));
    inv.record_fill(Side::Buy, Decimal::from_f64(101.5).unwrap(), d("0.5"));
    assert_eq!(inv.avg_entry_price, Decimal::from_f64(101.0).unwrap());
}

/// A fill the tracker can't hold (its entry price, position or realized `PnL` past a
/// `Decimal`) is refused and leaves the tracker as it was (security plugins-NV2).
#[test]
fn a_fill_that_would_overflow_is_refused_and_changes_nothing() {
    let mut inv = InventoryTracker::new();
    inv.record_fill(Side::Sell, d("100000"), d("4e23")).unwrap();
    let before = (inv.net_position, inv.avg_entry_price, inv.realized_pnl, inv.total_fills);
    let after = |inv: &InventoryTracker| {
        (inv.net_position, inv.avg_entry_price, inv.realized_pnl, inv.total_fills)
    };
    assert_eq!(inv.record_fill(Side::Sell, d("100001"), d("4e23")), None, "the entry price");
    assert_eq!(after(&inv), before);
    assert_eq!(inv.record_fill(Side::Sell, d("1"), Decimal::MAX), None, "the position");
    assert_eq!(after(&inv), before);
    assert_eq!(inv.record_fill(Side::Buy, d("-150000"), d("4e23")), None, "the realized PnL");
    assert_eq!(after(&inv), before);
    assert_eq!(inv.record_fill(Side::Buy, d("100000"), d("4e23")), Some(Decimal::ZERO));
    assert!(inv.is_flat());
}

#[test]
fn seed_from_position_sets_signed_inventory() {
    let mut inv = InventoryTracker::new();
    inv.seed_from_position(&Position {
        symbol: "BTC-USD".into(),
        side: Some(Side::Sell),
        size: d("0.5"),
        entry_price: d("100"),
        unrealized_pnl: Decimal::ZERO,
    });

    assert_eq!(inv.net_position, d("-0.5"));
    assert_eq!(inv.avg_entry_price, d("100"));
}

/// The bot stores a paper account's inventory as this JSON (`PaperAccountDoc`) and loads it back
/// after a pin bump: a renamed field or a changed type here breaks every stored paper account.
#[test]
fn the_stored_json_keeps_its_field_names_and_types() {
    let json =
        r#"{"net_position":"0.3","avg_entry_price":"1276","realized_pnl":"-1.5","total_fills":2}"#;
    let inv = InventoryTracker {
        net_position: d("0.3"),
        avg_entry_price: d("1276"),
        realized_pnl: d("-1.5"),
        total_fills: 2,
    };
    assert_eq!(serde_json::to_string(&inv).unwrap(), json);
    let back: InventoryTracker = serde_json::from_str(json).unwrap();
    assert_eq!(
        (back.net_position, back.avg_entry_price, back.realized_pnl, back.total_fills),
        (d("0.3"), d("1276"), d("-1.5"), 2)
    );
}
