#![expect(clippy::arithmetic_side_effects, reason = "test arithmetic")]

use super::*;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn params() -> Params {
    Params {
        notional_per_symbol: d("100"),
        top_k: 3,
        max_positions: 8,
        max_net_pct: d("0.2"),
        max_oi_pct: d("0.05"),
        max_leverage: d("2"),
        expected_hold_hours: 72,
        min_depth_usd: d("5000"),
    }
}

/// A flat, fresh, deep market with a 20bp round trip: carry over 72h is `9 × signal`.
fn sym(name: &str, signal: &str) -> SymbolView {
    SymbolView {
        symbol: name.into(),
        signal_8h: Some(d(signal)),
        round_trip_cost: Some(d("0.002")),
        mark: d("100"),
        oi_usd: Some(d("1000000")),
        depth_usd: Some(d("100000")),
        fresh: true,
        held_usd: Decimal::ZERO,
        entry_cost: None,
    }
}

fn held(name: &str, signal: &str, usd: &str) -> SymbolView {
    SymbolView { held_usd: d(usd), ..sym(name, signal) }
}

fn view(symbols: Vec<SymbolView>) -> MarketView {
    MarketView {
        symbols,
        equity: Some(d("10000")),
        foreign_gross_usd: Decimal::ZERO,
        params: params(),
    }
}

fn targets(v: &MarketView) -> Vec<(String, Decimal)> {
    decide(v).into_iter().map(|t| (t.symbol, t.notional_usd)).collect()
}

fn of(v: &MarketView, name: &str) -> Decimal {
    decide(v).into_iter().find(|t| t.symbol == name).unwrap().notional_usd
}

/// A pair that makes a basket neutral: one short and one long, both strong.
fn pair() -> Vec<SymbolView> {
    vec![sym("S1", "0.0006"), sym("L1", "-0.0006")]
}

#[test]
fn shorts_the_positive_signal_and_longs_the_negative_with_equal_legs() {
    let v = view(pair());
    assert_eq!(targets(&v), [("S1".into(), d("-100")), ("L1".into(), d("100"))]);
}

#[test]
fn one_target_per_symbol_in_view_order() {
    let mut syms = pair();
    syms.push(sym("FLAT", "0.00001"));
    let t = targets(&view(syms));
    assert_eq!(t.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>(), ["S1", "L1", "FLAT"]);
    assert_eq!(t[2].1, Decimal::ZERO);
}

#[test]
fn a_one_sided_basket_is_not_opened() {
    let v = view(vec![sym("S1", "0.0006"), sym("S2", "0.0005")]);
    assert!(targets(&v).iter().all(|(_, n)| n.is_zero()), "all shorts would be directional");
}

#[test]
fn entry_needs_carry_over_the_round_trip() {
    // A 18bp round trip: carry = 9 x signal is 0.0018 at 0.0002, equal to it.
    for (signal, enters) in [("0.00019", false), ("0.0002", false), ("0.00021", true)] {
        let mut s1 = sym("S1", signal);
        s1.round_trip_cost = Some(d("0.0018"));
        let v = view(vec![s1, sym("L1", "-0.0006")]);
        assert_eq!(of(&v, "S1") != Decimal::ZERO, enters, "signal {signal}");
    }
}

#[test]
fn hysteresis_holds_between_half_and_full_hurdle_but_does_not_enter() {
    // carry 0.0018 (9 x 0.0002): under the 0.002 hurdle, over half of it (0.001).
    let flat = view(vec![sym("S1", "0.0002"), sym("L1", "-0.0006")]);
    assert_eq!(of(&flat, "S1"), Decimal::ZERO, "does not enter");
    let holding = view(vec![held("S1", "0.0002", "-100"), sym("L1", "-0.0006")]);
    assert_eq!(of(&holding, "S1"), d("-100"), "keeps what it holds");
}

#[test]
fn exits_when_carry_falls_under_half_the_hurdle() {
    // carry 0.0009 < 0.001.
    let v = view(vec![held("S1", "0.0001", "-100"), sym("L1", "-0.0006")]);
    assert_eq!(of(&v, "S1"), Decimal::ZERO);
}

#[test]
fn a_held_leg_is_judged_on_the_cost_it_paid_not_on_todays_wider_book() {
    // Entered at a 40bp round trip: the hold band is carry >= 0.002. The book is 100bp wide
    // now (hurdle 0.005 on today's cost), carry 9 x 0.0004 = 0.0036: today's cost alone would
    // exit here and re-enter when the book narrows, paying the fees twice.
    let mut s1 = held("S1", "0.0004", "-100");
    s1.round_trip_cost = Some(d("0.01"));
    s1.entry_cost = Some(d("0.004"));
    let v = view(vec![s1, sym("L1", "-0.0006")]);
    assert_eq!(of(&v, "S1"), d("-100"), "kept: above half the entry cost");
}

#[test]
fn a_held_leg_exits_under_half_its_entry_cost_whatever_the_book_looks_like_now() {
    // Paid 100bp to get in: carry 0.0036 is under half of that (0.005). A narrow book today
    // (hurdle 0.001) does not reset the band.
    let mut s1 = held("S1", "0.0004", "-100");
    s1.round_trip_cost = Some(d("0.002"));
    s1.entry_cost = Some(d("0.01"));
    let v = view(vec![s1, sym("L1", "-0.0006")]);
    assert_eq!(of(&v, "S1"), Decimal::ZERO);
}

#[test]
fn an_unknown_entry_cost_falls_back_to_todays_round_trip() {
    let mut s1 = held("S1", "0.0004", "-100");
    s1.round_trip_cost = Some(d("0.01"));
    let v = view(vec![s1, sym("L1", "-0.0006")]);
    assert_eq!(of(&v, "S1"), Decimal::ZERO, "0.0036 < half of 0.01");
}

#[test]
fn a_known_entry_cost_still_judges_the_hold_when_todays_cost_is_unknown() {
    let mut s1 = held("S1", "0.0001", "-100");
    s1.round_trip_cost = None;
    s1.entry_cost = Some(d("0.004"));
    let v = view(vec![s1, sym("L1", "-0.0006")]);
    assert_eq!(of(&v, "S1"), Decimal::ZERO, "carry 0.0009 < half of what it cost to enter");
}

#[test]
fn exits_when_the_signal_flips_sign_and_does_not_reverse_in_one_tick() {
    let v = view(vec![held("S1", "-0.0009", "-100"), held("L1", "0.0009", "100")]);
    assert_eq!(of(&v, "S1"), Decimal::ZERO);
    assert_eq!(of(&v, "L1"), Decimal::ZERO);
}

#[test]
fn a_zero_signal_exits() {
    let v = view(vec![held("S1", "0", "-100"), held("L1", "-0.0006", "100")]);
    assert_eq!(of(&v, "S1"), Decimal::ZERO);
}

#[test]
fn unknown_signal_blocks_entry_but_not_hold() {
    let mut a = held("S1", "0.0006", "-100");
    a.signal_8h = None;
    let mut b = sym("L1", "-0.0006");
    b.signal_8h = None;
    let v = view(vec![a, b, sym("S2", "0.0006"), sym("L2", "-0.0006")]);
    assert_eq!(of(&v, "S1"), d("-100"), "held stays");
    assert_eq!(of(&v, "L1"), Decimal::ZERO, "no entry");
}

#[test]
fn unknown_cost_blocks_entry_but_a_hold_goes_on_unless_the_signal_flips() {
    let mut a = held("S1", "0.0001", "-100");
    a.round_trip_cost = None;
    let mut b = sym("L1", "-0.0006");
    b.round_trip_cost = None;
    let mut c = held("S2", "-0.0006", "-100");
    c.round_trip_cost = None;
    let v = view(vec![a, b, c, sym("S3", "0.0006"), sym("L3", "-0.0006")]);
    assert_eq!(of(&v, "S1"), d("-100"));
    assert_eq!(of(&v, "L1"), Decimal::ZERO);
    assert_eq!(of(&v, "S2"), Decimal::ZERO, "flipped");
}

#[test]
fn unknown_equity_blocks_entries_but_keeps_and_exits_holds() {
    let mut v = view(vec![
        held("S1", "0.0006", "-100"),
        held("L1", "0.0006", "100"),
        sym("S2", "0.0006"),
        sym("L2", "-0.0006"),
    ]);
    v.equity = None;
    assert_eq!(of(&v, "S1"), d("-100"));
    assert_eq!(of(&v, "L1"), Decimal::ZERO, "flipped hold still exits");
    assert_eq!(of(&v, "S2"), Decimal::ZERO);
    assert_eq!(of(&v, "L2"), Decimal::ZERO);
}

#[test]
fn stale_data_blocks_entry_not_hold() {
    let mut a = held("S1", "0.0006", "-100");
    a.fresh = false;
    let mut b = sym("L1", "-0.0006");
    b.fresh = false;
    let v = view(vec![a, b, sym("S2", "0.0006"), sym("L2", "-0.0006")]);
    assert_eq!(of(&v, "S1"), d("-100"));
    assert_eq!(of(&v, "L1"), Decimal::ZERO);
}

#[test]
fn thin_or_unknown_depth_blocks_entry() {
    let mut thin = sym("S1", "0.0006");
    thin.depth_usd = Some(d("4999"));
    let mut unknown = sym("L1", "-0.0006");
    unknown.depth_usd = None;
    let v = view(vec![thin, unknown, sym("S2", "0.0006"), sym("L2", "-0.0006")]);
    assert_eq!(of(&v, "S1"), Decimal::ZERO);
    assert_eq!(of(&v, "L1"), Decimal::ZERO);
    assert_eq!(of(&v, "S2"), d("-100"));
    let mut edge = sym("S3", "0.0006");
    edge.depth_usd = Some(d("5000"));
    assert_eq!(of(&view(vec![edge, sym("L3", "-0.0006")]), "S3"), d("-100"), "exactly the minimum");
}

#[test]
fn open_interest_clips_the_leg_and_a_clip_under_half_is_no_entry() {
    // 5% of 1,600 = 80: a clipped leg of 80 (>= half of 100).
    let mut s = sym("S1", "0.0006");
    s.oi_usd = Some(d("1600"));
    assert_eq!(of(&view(vec![s, sym("L1", "-0.0006")]), "S1"), d("-80"));
    // 5% of 800 = 40 < 50.
    let mut s = sym("S1", "0.0006");
    s.oi_usd = Some(d("800"));
    assert_eq!(of(&view(vec![s, sym("L1", "-0.0006")]), "S1"), Decimal::ZERO);
    let mut s = sym("S1", "0.0006");
    s.oi_usd = None;
    assert_eq!(of(&view(vec![s, sym("L1", "-0.0006")]), "S1"), Decimal::ZERO, "unknown OI");
    let mut s = sym("S1", "0.0006");
    s.mark = Decimal::ZERO;
    assert_eq!(of(&view(vec![s, sym("L1", "-0.0006")]), "S1"), Decimal::ZERO, "unknown mark");
}

#[test]
fn top_k_limits_new_legs_per_side_best_first() {
    let mut p = params();
    p.top_k = 2;
    p.max_net_pct = d("1");
    let mut v = view(vec![
        sym("S1", "0.0005"),
        sym("S2", "0.0009"),
        sym("S3", "0.0007"),
        sym("L1", "-0.0008"),
    ]);
    v.params = p;
    assert_eq!(of(&v, "S2"), d("-100"));
    assert_eq!(of(&v, "S3"), d("-100"));
    assert_eq!(of(&v, "S1"), Decimal::ZERO, "third short: outside the top 2");
    assert_eq!(of(&v, "L1"), d("100"));
}

#[test]
fn held_legs_count_against_top_k_and_are_never_displaced() {
    let mut p = params();
    p.top_k = 1;
    p.max_net_pct = d("1");
    let mut v = view(vec![held("S1", "0.0004", "-100"), sym("S2", "0.0009"), sym("L1", "-0.0008")]);
    v.params = p;
    assert_eq!(of(&v, "S1"), d("-100"), "kept though S2 ranks higher");
    assert_eq!(of(&v, "S2"), Decimal::ZERO);
}

#[test]
fn max_positions_bounds_open_symbols() {
    let mut p = params();
    p.max_positions = 3;
    p.max_net_pct = d("1");
    let mut v = view(vec![
        sym("S1", "0.0009"),
        sym("L1", "-0.0009"),
        sym("S2", "0.0008"),
        sym("L2", "-0.0008"),
    ]);
    v.params = p;
    let open = decide(&v).iter().filter(|t| !t.notional_usd.is_zero()).count();
    assert!(open <= 3, "{open}");
    assert!(open >= 2);
}

#[test]
fn leverage_budget_caps_gross_notional_including_foreign() {
    let mut p = params();
    p.max_net_pct = d("1");
    // equity 100 x 2 = 200 gross, minus 50 foreign: room for one 100 leg.
    let mut v = view(pair());
    v.params = p;
    v.equity = Some(d("100"));
    v.foreign_gross_usd = d("50");
    let gross: Decimal = decide(&v).iter().map(|t| t.notional_usd.abs()).sum();
    assert!(gross <= d("150"), "{gross}");
    // One leg alone is lopsided: with the bound back on, nothing opens.
    let mut v = view(pair());
    v.equity = Some(d("100"));
    v.foreign_gross_usd = d("50");
    assert!(decide(&v).iter().all(|t| t.notional_usd.is_zero()));
}

#[test]
fn held_positions_are_kept_even_over_the_leverage_budget() {
    let mut v = view(vec![held("S1", "0.0006", "-100"), held("L1", "-0.0006", "100")]);
    v.equity = Some(d("10"));
    assert_eq!(targets(&v), [("S1".into(), d("-100")), ("L1".into(), d("100"))]);
}

#[test]
fn a_partial_fill_is_topped_up_but_a_full_one_left_alone() {
    let v = view(vec![held("S1", "0.0006", "-60"), held("L1", "-0.0006", "100")]);
    assert_eq!(of(&v, "S1"), d("-100"), "topped up to the target");
    assert_eq!(of(&v, "L1"), d("100"));
    let v = view(vec![held("S1", "0.0006", "-95"), held("L1", "-0.0006", "130")]);
    assert_eq!(of(&v, "S1"), d("-95"), "within 10% of target");
    assert_eq!(of(&v, "L1"), d("130"), "a leg that grew is not trimmed");
}

#[test]
fn neutrality_drops_the_lowest_ranked_leg_on_the_heavy_side() {
    // Two shorts and one long: 100/300 apart > 0.2. The weaker short goes.
    let v = view(vec![sym("S1", "0.0009"), sym("S2", "0.0005"), sym("L1", "-0.0007")]);
    assert_eq!(of(&v, "S1"), d("-100"));
    assert_eq!(of(&v, "S2"), Decimal::ZERO);
    assert_eq!(of(&v, "L1"), d("100"));
}

#[test]
fn neutrality_allows_three_against_two_at_the_default_bound() {
    let v = view(vec![
        sym("S1", "0.0009"),
        sym("S2", "0.0008"),
        sym("S3", "0.0007"),
        sym("L1", "-0.0009"),
        sym("L2", "-0.0008"),
    ]);
    let t = decide(&v);
    assert_eq!(t.iter().filter(|t| t.notional_usd < Decimal::ZERO).count(), 3);
    assert_eq!(t.iter().filter(|t| t.notional_usd > Decimal::ZERO).count(), 2);
}

#[test]
fn a_lopsided_hold_is_rebalanced_only_by_adding_to_the_light_side() {
    let v = view(vec![
        held("S1", "0.0006", "-100"),
        held("S2", "0.0006", "-100"),
        sym("S3", "0.0009"),
        sym("L1", "-0.0006"),
    ]);
    assert_eq!(of(&v, "S3"), Decimal::ZERO, "never adds to the heavy side");
    assert_eq!(of(&v, "L1"), d("100"));
    assert_eq!(of(&v, "S1"), d("-100"));
}

#[test]
fn nothing_to_do_is_all_zero() {
    let v = view(vec![sym("A", "0.00001"), sym("B", "-0.00001")]);
    assert!(decide(&v).iter().all(|t| t.notional_usd.is_zero()));
    assert_eq!(decide(&view(vec![])), vec![]);
}

#[path = "decide_property_tests.rs"]
mod property_tests;
