//! The stops, the start over positions, wind-down.

use bb_strategy_sdk::testing::d;
use bb_strategy_sdk::{Decimal, Intent, Position, Side, StrategyAdapter, StrategyReport};

use super::harness::{Sim, T0, settings};

fn ready(extra: &str) -> Sim {
    let mut sim = Sim::new(settings(extra), &["S1", "L1"]);
    sim.start().unwrap();
    sim
}

#[test]
fn a_daily_loss_flattens_and_halts_until_the_next_utc_day() {
    let mut sim = ready("");
    sim.tick();
    assert_ne!(sim.held("S1"), Decimal::ZERO);
    sim.venue.equity = Some(d("9970"));
    sim.advance(61);
    sim.tick();
    assert_eq!((sim.held("S1"), sim.held("L1")), (Decimal::ZERO, Decimal::ZERO), "flat");
    assert!(sim.orders().iter().rev().take(2).all(|o| o.reduce_only));
    assert!(sim.strategy.report().headline().starts_with("halted"));
    let n = sim.orders().len();
    sim.advance(61);
    sim.tick();
    assert_eq!(sim.orders().len(), n, "halted: no entry");
    // The next UTC day lifts it, and the allowance restarts from that equity.
    sim.market.now_ms = T0 + 86_400_000 + 1_000;
    sim.tick();
    assert!(sim.orders().len() > n, "trading again");
    assert!(!sim.strategy.report().headline().starts_with("halted"));
}

#[test]
fn a_clock_that_goes_backwards_does_not_reset_the_loss_baseline() {
    let mut sim = ready("");
    sim.tick();
    sim.venue.equity = Some(d("9970"));
    sim.advance(61);
    sim.tick();
    assert!(sim.strategy.report().headline().starts_with("halted"));
    sim.market.now_ms = T0 - 86_400_000;
    sim.tick();
    assert!(sim.strategy.report().headline().starts_with("halted"), "an earlier day lifts nothing");
}

#[test]
fn a_margin_ratio_at_the_flatten_line_flattens_and_halts() {
    let mut sim = ready("");
    sim.tick();
    sim.venue.margin = Some(d("0.8"));
    sim.advance(61);
    sim.tick();
    assert_eq!((sim.held("S1"), sim.held("L1")), (Decimal::ZERO, Decimal::ZERO));
    assert!(sim.strategy.report().headline().starts_with("halted"));
}

fn held_start(sim: &mut Sim) {
    sim.venue.held.insert("S1".into(), d("-2"));
    sim.market.legs[0].positions = Some(vec![Position {
        symbol: "S1".into(),
        side: Some(Side::Sell),
        size: d("2"),
        entry_price: d("100"),
        unrealized_pnl: Decimal::ZERO,
    }]);
}

#[test]
fn a_start_over_a_position_it_did_not_open_is_refused_unless_adopted() {
    let mut sim = Sim::new(settings(""), &["S1", "L1"]);
    held_start(&mut sim);
    let e = sim.start().unwrap_err();
    assert!(e.0.contains("adopt_existing_positions"), "{e}");
    let mut sim = Sim::new(settings(r#""adopt_existing_positions": true"#), &["S1", "L1"]);
    held_start(&mut sim);
    assert!(sim.start().is_ok());
}

#[test]
fn when_the_start_cannot_read_positions_the_first_tick_refuses_and_trades_nothing() {
    let mut sim = Sim::new(settings(""), &["S1", "L1"]);
    held_start(&mut sim);
    sim.market.legs[0].positions = None;
    sim.start().unwrap();
    sim.tick();
    assert_eq!(sim.orders(), []);
    assert!(sim.strategy.report().headline().starts_with("not trading"));
    // Its wind-down leaves a position that is not its own.
    assert_eq!(sim.strategy.stop(&sim.market.view()), []);
}

#[test]
fn wind_down_flattens_each_held_universe_symbol_and_unknown_is_every_symbol() {
    let mut sim = ready("");
    sim.tick();
    let intents = sim.strategy.stop(&sim.market.view());
    let Some(Intent::Leg { leg, intent }) = intents.first() else { panic!("{intents:?}") };
    assert_eq!(leg, "universe");
    let Intent::Flatten { symbols } = &**intent else { panic!("{intent:?}") };
    assert_eq!(symbols, &["L1", "S1"]);
    sim.market.legs[0].positions = None;
    let intents = sim.strategy.stop(&sim.market.view());
    assert_eq!(intents.len(), 1, "unknown is never flat");
    sim.market.legs[0].positions = Some(Vec::new());
    assert!(sim.strategy.stop(&sim.market.view()).is_empty(), "flat: nothing to close");
}

#[test]
fn a_lopsided_book_sheds_its_weakest_heavy_leg_after_the_watchdog_ticks() {
    let mut sim = Sim::new(settings(r#""adopt_existing_positions": true"#), &["S1", "L1"]);
    sim.venue.held.insert("S1".into(), d("-1"));
    sim.venue.held.insert("L1".into(), d("0.1"));
    sim.market.legs[0].positions = Some(sim.venue.position_list());
    sim.start().unwrap();
    // The first tick finds the book lopsided over positions it adopted: shed at once.
    sim.tick();
    assert_eq!(sim.held("S1"), Decimal::ZERO, "the short side was the heavy one");
    assert_eq!(sim.held("L1"), d("0.1"));
}
