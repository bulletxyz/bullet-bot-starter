use bb_strategy_sdk::targets::Leg;

use super::*;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn params() -> Params {
    Params {
        notional_per_symbol: d("100"),
        max_positions: 8,
        max_oi_pct: d("0.10"),
        max_leverage: d("2"),
        expected_hold_hours: 72,
    }
}

/// Flat symbol, mark $10, deep OI.
fn sym(name: &str, signal: Option<&str>, cost: Option<&str>) -> SymbolView {
    SymbolView {
        symbol: name.to_string(),
        signal_8h: signal.map(d),
        round_trip_cost: cost.map(d),
        mark: d("10"),
        bullet_oi_usd: Some(d("1000000")),
        leg_bullet: Leg::default(),
        leg_hl: Leg::default(),
    }
}

/// Held with `bullet_base` on Bullet (mirror on HL); $10 mark.
fn held(name: &str, bullet_base: &str, signal: Option<&str>, cost: Option<&str>) -> SymbolView {
    let base = d(bullet_base);
    SymbolView { leg_bullet: Leg { base }, leg_hl: Leg { base: -base }, ..sym(name, signal, cost) }
}

fn view(symbols: Vec<SymbolView>) -> MarketView {
    MarketView {
        symbols,
        equity_bullet: Some(d("10000")),
        equity_hl: Some(d("10000")),
        foreign_notional_bullet: Decimal::ZERO,
        foreign_notional_hl: Decimal::ZERO,
        params: params(),
    }
}

fn target<'a>(targets: &'a [Target], name: &str) -> &'a Target {
    targets.iter().find(|t| t.symbol == name).unwrap()
}

fn notional(targets: &[Target], name: &str) -> Decimal {
    target(targets, name).notional_usd
}

#[test]
fn positive_spread_targets_short_bullet_at_full_notional() {
    // Carry 0.0005 / 8 × 72 = 0.0045 > 0.0010.
    let targets = decide(&view(vec![sym("A", Some("0.0005"), Some("0.0010"))]));
    assert_eq!(
        targets,
        vec![Target {
            symbol: "A".into(),
            direction: Direction::ShortBullet,
            notional_usd: d("100")
        }]
    );
    let targets = decide(&view(vec![sym("A", Some("-0.0005"), Some("0.0010"))]));
    assert_eq!(target(&targets, "A").direction, Direction::LongBullet);
    assert_eq!(notional(&targets, "A"), d("100"));
}

#[test]
fn below_hurdle_flat_symbol_gets_zero() {
    // Carry 0.0001 / 8 × 72 = 0.0009 < 0.0010.
    let targets = decide(&view(vec![sym("A", Some("0.0001"), Some("0.0010"))]));
    assert_eq!(targets.len(), 1);
    assert_eq!(notional(&targets, "A"), Decimal::ZERO);
}

#[test]
fn held_symbol_below_hurdle_holds_but_never_grows() {
    let targets = decide(&view(vec![
        held("A", "-5", Some("0.0001"), Some("0.0010")),
        held("B", "-15", Some("0.0001"), Some("0.0010")),
    ]));
    // $50 held: kept, not grown to 100.
    assert_eq!(target(&targets, "A").direction, Direction::ShortBullet);
    assert_eq!(notional(&targets, "A"), d("50"));
    // $150 held: trimmed to the cap.
    assert_eq!(notional(&targets, "B"), d("100"));
}

#[test]
fn sign_flip_against_held_direction_exits() {
    let targets = decide(&view(vec![
        held("A", "-5", Some("-0.0005"), Some("0.0010")),
        // Flipped but tiny, below the hurdle: still an exit.
        held("B", "5", Some("0.00001"), Some("0.0010")),
    ]));
    assert_eq!(notional(&targets, "A"), Decimal::ZERO);
    assert_eq!(notional(&targets, "B"), Decimal::ZERO);
}

#[test]
fn unknown_signal_on_held_symbol_holds() {
    let targets = decide(&view(vec![held("A", "-5", None, Some("0.0010"))]));
    assert_eq!(target(&targets, "A").direction, Direction::ShortBullet);
    assert_eq!(notional(&targets, "A"), d("50"));
}

#[test]
fn unknown_cost_blocks_entry_but_not_hold() {
    let targets = decide(&view(vec![
        sym("FLAT", Some("0.0005"), None),
        held("HELD", "-5", Some("0.0005"), None),
    ]));
    assert_eq!(notional(&targets, "FLAT"), Decimal::ZERO);
    assert_eq!(notional(&targets, "HELD"), d("50"));
}

#[test]
fn oi_cap_limits_target() {
    let mut s = sym("A", Some("0.0005"), Some("0.0010"));
    s.bullet_oi_usd = Some(d("500"));
    let mut unknown = sym("B", Some("0.0005"), Some("0.0010"));
    unknown.bullet_oi_usd = None;
    let mut held_unknown = held("C", "-5", Some("0.0005"), Some("0.0010"));
    held_unknown.bullet_oi_usd = None;
    let targets = decide(&view(vec![s, unknown, held_unknown]));
    assert_eq!(notional(&targets, "A"), d("50"));
    // Unknown OI: no entry, and a held position is neither grown nor trimmed.
    assert_eq!(notional(&targets, "B"), Decimal::ZERO);
    assert_eq!(notional(&targets, "C"), d("50"));
}

#[test]
fn leverage_budget_fills_best_carry_first() {
    // Budget 2 × 125 = $250 per venue.
    let mut v = view(vec![
        sym("LOW", Some("0.0005"), Some("0.0010")),
        sym("HIGH", Some("0.0010"), Some("0.0010")),
        sym("MID", Some("0.0008"), Some("0.0010")),
    ]);
    v.equity_bullet = Some(d("125"));
    v.equity_hl = Some(d("500"));
    let targets = decide(&v);
    assert_eq!(notional(&targets, "HIGH"), d("100"));
    assert_eq!(notional(&targets, "MID"), d("100"));
    assert_eq!(notional(&targets, "LOW"), d("50"));
    // View order is kept.
    let names: Vec<&str> = targets.iter().map(|t| t.symbol.as_str()).collect();
    assert_eq!(names, ["LOW", "HIGH", "MID"]);
}

#[test]
fn max_positions_bounds_count() {
    let mut v = view(vec![
        sym("LOW", Some("0.0005"), Some("0.0010")),
        held("HELD", "-10", Some("0.0002"), Some("0.0010")),
        sym("HIGH", Some("0.0010"), Some("0.0010")),
    ]);
    v.params.max_positions = 2;
    let targets = decide(&v);
    // The held position keeps its slot even with the weakest carry.
    assert_eq!(notional(&targets, "HELD"), d("100"));
    assert_eq!(notional(&targets, "HIGH"), d("100"));
    assert_eq!(notional(&targets, "LOW"), Decimal::ZERO);
    assert_eq!(targets.len(), 3);
}

#[test]
fn foreign_notional_eats_budget() {
    // HL: 2 × 100 − 150 = $50 left; Bullet has plenty.
    let mut v = view(vec![sym("A", Some("0.0005"), Some("0.0010"))]);
    v.equity_hl = Some(d("100"));
    v.foreign_notional_hl = d("150");
    assert_eq!(notional(&decide(&v), "A"), d("50"));
    v.foreign_notional_hl = d("250");
    assert_eq!(notional(&decide(&v), "A"), Decimal::ZERO);
}

#[test]
fn unknown_equity_blocks_grow() {
    let mut v = view(vec![
        sym("FLAT", Some("0.0005"), Some("0.0010")),
        held("HELD", "-5", Some("0.0005"), Some("0.0010")),
    ]);
    v.equity_hl = None;
    let targets = decide(&v);
    assert_eq!(notional(&targets, "FLAT"), Decimal::ZERO);
    // Kept at $50: not grown, not trimmed.
    assert_eq!(notional(&targets, "HELD"), d("50"));
}

#[test]
fn decide_with_zero_mark_holds_and_never_grows() {
    let mut flat = sym("FLAT", Some("0.0005"), Some("0.0010"));
    flat.mark = Decimal::ZERO;
    let mut hold = held("HELD", "-5", Some("0.0005"), Some("0.0010"));
    hold.mark = Decimal::ZERO;
    let mut hold_no_oi = held("NOOI", "5", None, None);
    hold_no_oi.mark = Decimal::ZERO;
    hold_no_oi.bullet_oi_usd = None;
    let targets = decide(&view(vec![flat, hold, hold_no_oi]));
    assert_eq!(notional(&targets, "FLAT"), Decimal::ZERO, "no entry without a mark");
    // A held symbol is not exited by the gap; with no current USD size known, the
    // hold is bounded by the cap.
    assert_eq!(target(&targets, "HELD").direction, Direction::ShortBullet);
    assert_eq!(notional(&targets, "HELD"), d("100"));
    assert_eq!(target(&targets, "NOOI").direction, Direction::LongBullet);
    assert_eq!(notional(&targets, "NOOI"), d("100"));
}
