use super::*;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn caps() -> Caps {
    Caps {
        notional_per_symbol: d("100"),
        max_positions: 8,
        max_leverage: d("2"),
        max_oi_pct: d("0.05"),
    }
}

/// Flat at mark 10, step 0.1, plenty of OI, fresh.
fn row(symbol: &str, held_base: &str, target_usd: &str) -> Row {
    Row {
        symbol: symbol.into(),
        mark: d("10"),
        held_base: d(held_base),
        step: Some(d("0.1")),
        oi_base: Some(d("1000000")),
        fresh: true,
        target_usd: d(target_usd),
    }
}

fn run(rows: &[Row]) -> Vec<Action> {
    plan(rows, Some(d("10000")), Decimal::ZERO, &caps())
}

fn act(a: &Action) -> (&str, Side, Decimal, bool) {
    (a.symbol.as_str(), a.side, a.qty, a.reduce_only)
}

#[test]
fn enters_long_and_short_at_the_target_size() {
    let out = run(&[row("A", "0", "100"), row("B", "0", "-100")]);
    assert_eq!(act(&out[0]), ("A", Side::Buy, d("10"), false));
    assert_eq!(act(&out[1]), ("B", Side::Sell, d("10"), false));
}

#[test]
fn a_zero_target_exits_the_whole_position_reduce_only() {
    let out = run(&[row("A", "3.5", "0"), row("B", "-2", "0")]);
    assert_eq!(act(&out[0]), ("A", Side::Sell, d("3.5"), true));
    assert_eq!(act(&out[1]), ("B", Side::Buy, d("2"), true));
}

#[test]
fn exits_go_first_and_need_no_fresh_data_equity_or_step() {
    let mut stale = row("A", "3", "0");
    stale.fresh = false;
    stale.step = None;
    stale.oi_base = None;
    let out = plan(&[row("B", "0", "100"), stale], None, Decimal::ZERO, &caps());
    assert_eq!(out.len(), 1);
    assert_eq!(act(&out[0]), ("A", Side::Sell, d("3"), true));
}

#[test]
fn a_target_against_the_position_only_exits_it() {
    let out = run(&[row("A", "3", "-100")]);
    assert_eq!(out.len(), 1);
    assert_eq!(act(&out[0]), ("A", Side::Sell, d("3"), true));
}

#[test]
fn the_leg_is_clamped_to_notional_per_symbol() {
    let out = run(&[row("A", "0", "5000")]);
    assert_eq!(act(&out[0]), ("A", Side::Buy, d("10"), false));
}

#[test]
fn the_leg_is_clipped_to_the_open_interest_share() {
    let mut r = row("A", "0", "100");
    r.oi_base = Some(d("100"));
    // 5% of 100 base x mark 10 = 50 USD = 5 base.
    assert_eq!(act(&run(&[r])[0]), ("A", Side::Buy, d("5"), false));
}

#[test]
fn unknown_or_stale_inputs_block_growth() {
    let mut no_oi = row("A", "0", "100");
    no_oi.oi_base = None;
    let mut no_step = row("A", "0", "100");
    no_step.step = None;
    let mut stale = row("A", "0", "100");
    stale.fresh = false;
    let mut no_mark = row("A", "0", "100");
    no_mark.mark = Decimal::ZERO;
    for r in [no_oi, no_step, stale, no_mark] {
        assert_eq!(run(&[r]), vec![]);
    }
    assert!(plan(&[row("A", "0", "100")], None, Decimal::ZERO, &caps()).is_empty(), "no equity");
}

#[test]
fn leverage_room_clips_and_then_blocks() {
    // 2 x 100 equity = 200 gross; 80 foreign; 60 held: 60 USD left = 6 base.
    let rows = [row("H", "6", "60"), row("A", "0", "100"), row("B", "0", "-100")];
    let out = plan(&rows, Some(d("100")), d("80"), &caps());
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(act(&out[0]), ("A", Side::Buy, d("6"), false));
}

#[test]
fn max_positions_bounds_new_symbols_but_not_topping_up_a_held_one() {
    let mut c = caps();
    c.max_positions = 1;
    let rows = [row("H", "5", "100"), row("N", "0", "-100")];
    let out = plan(&rows, Some(d("10000")), Decimal::ZERO, &c);
    assert_eq!(out.len(), 1);
    assert_eq!(act(&out[0]), ("H", Side::Buy, d("5"), false), "top-up allowed, new symbol not");
}

#[test]
fn one_new_symbol_per_side_per_tick() {
    let rows =
        [row("A", "0", "100"), row("B", "0", "100"), row("C", "0", "-100"), row("D", "0", "-100")];
    let out = run(&rows);
    let names: Vec<_> = out.iter().map(|a| a.symbol.as_str()).collect();
    assert_eq!(names, ["A", "C"]);
}

#[test]
fn small_gaps_are_not_traded() {
    // Held 9.5 of a 10 target: 5% short, under the 10% threshold.
    assert_eq!(run(&[row("A", "9.5", "100")]), vec![]);
    assert_eq!(run(&[row("A", "8", "100")]).len(), 1);
}

#[test]
fn quantities_round_down_to_the_step_and_below_one_step_is_nothing() {
    let mut r = row("A", "0", "100");
    r.mark = d("30");
    r.step = Some(d("0.1"));
    assert_eq!(act(&run(&[r])[0]), ("A", Side::Buy, d("3.3"), false));
    let mut tiny = row("A", "0", "100");
    tiny.step = Some(d("100"));
    assert_eq!(run(&[tiny]), vec![]);
}

#[test]
fn a_leg_that_grew_past_the_target_is_not_trimmed() {
    assert_eq!(run(&[row("A", "15", "100")]), vec![]);
}
