use bb_strategy_sdk::Side;
use rust_decimal::Decimal;

use super::*;
use crate::config::GridConfig;
use crate::plan::{HardLimits, Market, resolve};

fn plan(lower: i64, upper: i64, n: usize, anchor: Decimal) -> Plan {
    let cfg: GridConfig = serde_json::from_value(serde_json::json!({
        "symbol": "BTC-USD", "lower_price": lower.to_string(), "upper_price": upper.to_string(),
        "num_levels": n, "order_size": "1", "anchor_price": anchor.to_string(),
        "amount_usd": "100000"
    }))
    .unwrap();
    let mid = Decimal::from(lower + upper) / Decimal::TWO;
    resolve(&cfg, Market { mid, tick: None, lot: None }, None, HardLimits::default()).unwrap()
}

fn state(lower: i64, upper: i64, n: usize, anchor: Decimal) -> GridState {
    let mut s = GridState::new();
    s.compute_levels(&plan(lower, upper, n, anchor), None);
    s
}

fn count(s: &GridState, side: Side) -> usize {
    s.levels.iter().filter(|l| l.side == Some(side)).count()
}

#[test]
fn compute_levels_uniform_spacing() {
    let s = state(70, 80, 11, Decimal::from(75));
    assert_eq!(s.levels.len(), 11);
    assert_eq!(s.levels[0].price, Decimal::from(70));
    assert_eq!(s.levels[10].price, Decimal::from(80));
}

#[test]
fn the_level_at_the_anchor_is_left_empty_and_the_rest_split_by_it() {
    // Anchor 72: 70 and 71 buy, 72 empty, 73..80 sell.
    let s = state(70, 80, 11, Decimal::from(72));
    assert_eq!((count(&s, Side::Buy), count(&s, Side::Sell)), (2, 8));
    assert_eq!((s.levels[2].side, s.levels[2].state), (None, LevelState::Dormant));
    assert!(s.levels.iter().filter(|l| l.side.is_some()).all(|l| l.state == LevelState::Pending));
}

fn full_fill(s: &mut GridState, idx: usize) -> FillOutcome {
    let size = Decimal::ONE;
    s.record_fill(idx, size, size)
}

#[test]
fn full_fill_rearms_dormant_level_above() {
    let mut s = state(70, 80, 11, Decimal::from(75));
    // Level 4 Active, its buy fills; level 5 (the anchor's) is dormant, so the re-arm lands.
    s.levels[4].state = LevelState::Active;
    assert_eq!(full_fill(&mut s, 4), FillOutcome::Complete { rearm: Some((5, Side::Sell)) });
    assert_eq!((s.levels[4].state, s.levels[4].side), (LevelState::Dormant, None));
    assert_eq!((s.levels[5].state, s.levels[5].side), (LevelState::Pending, Some(Side::Sell)));
}

#[test]
fn full_fill_sell_rearms_level_below() {
    let mut s = state(70, 80, 11, Decimal::from(75));
    s.levels[6].state = LevelState::Active;
    assert_eq!(full_fill(&mut s, 6), FillOutcome::Complete { rearm: Some((5, Side::Buy)) });
}

#[test]
fn full_fill_skips_rearm_when_adjacent_not_dormant() {
    let mut s = state(70, 80, 11, Decimal::from(75));
    s.levels[3].state = LevelState::Active;
    // Level 4 is Pending from init, not Dormant — re-arm should skip.
    assert_eq!(full_fill(&mut s, 3), FillOutcome::Complete { rearm: None });
    assert_eq!(s.levels[3].state, LevelState::Dormant);
}

#[test]
fn full_fill_at_top_edge_has_no_rearm() {
    let mut s = state(70, 80, 11, Decimal::from(75));
    s.levels[10].state = LevelState::Active;
    s.levels[10].side = Some(Side::Buy);
    assert_eq!(full_fill(&mut s, 10), FillOutcome::Complete { rearm: None });
}

#[test]
fn partial_fill_accumulates_without_rearm() {
    let mut s = state(70, 80, 11, Decimal::from(75));
    s.levels[4].state = LevelState::Active;
    let half = Decimal::new(5, 1);
    let size = Decimal::ONE;
    assert_eq!(s.record_fill(4, half, size), FillOutcome::Partial);
    assert_eq!(s.levels[4].state, LevelState::Active);
    assert_eq!(s.levels[4].filled_qty, half);
    assert_eq!(s.levels[5].state, LevelState::Dormant);
    assert_eq!(
        s.record_fill(4, half, size),
        FillOutcome::Complete { rearm: Some((5, Side::Sell)) }
    );
    assert_eq!((s.levels[4].state, s.levels[4].filled_qty), (LevelState::Dormant, Decimal::ZERO));
}

#[test]
fn full_round_trip_generates_complementary_rearm() {
    // Anchor 75.5, halfway: 74, 75 buy; 76, 77, 78 sell; none empty.
    let mut s = state(74, 78, 5, Decimal::new(755, 1));
    s.levels[1].state = LevelState::Active;
    s.levels[2].state = LevelState::Active;
    // Buy at 75 fills: level 2 is Active, so nothing re-arms.
    assert_eq!(full_fill(&mut s, 1), FillOutcome::Complete { rearm: None });
    // Sell at 76 fills: level 1 is now Dormant, the re-arm lands as a buy.
    assert_eq!(full_fill(&mut s, 2), FillOutcome::Complete { rearm: Some((1, Side::Buy)) });
    assert_eq!((s.levels[1].state, s.levels[1].side), (LevelState::Pending, Some(Side::Buy)));
}

/// A long position is closed by the sells nearest the price, as far as it goes; the rest of the
/// sells and every buy open.
#[test]
fn closing_orders_are_the_nearest_against_the_position_as_far_as_it_goes() {
    let s = state(70, 80, 11, Decimal::from(75));
    let closing = s.closing(Decimal::from(2), Decimal::ONE);
    let idx: Vec<usize> = (0..11).filter(|&i| closing[i]).collect();
    assert_eq!(idx, [6, 7], "the two lowest sells close a long of 2");
    let short = s.closing(Decimal::from(-1), Decimal::ONE);
    assert_eq!((0..11).filter(|&i| short[i]).collect::<Vec<_>>(), [4], "the highest buy");
    assert!(s.closing(Decimal::ZERO, Decimal::ONE).iter().all(|c| !c), "flat: all open");
}

#[test]
fn a_paused_level_keeps_its_side_and_comes_back_pending() {
    let mut s = state(70, 80, 11, Decimal::from(75));
    s.levels[3].state = LevelState::Active;
    s.levels[3].client_id = Some("c".into());
    s.pause(3);
    assert_eq!((s.levels[3].state, s.levels[3].side), (LevelState::Paused, Some(Side::Buy)));
    assert!(s.levels[3].client_id.is_none());
    s.unpause_all();
    assert_eq!(s.levels[3].state, LevelState::Pending);
    s.clear_all();
    assert!(s.levels.iter().all(|l| l.state == LevelState::Dormant && l.side.is_none()));
}
