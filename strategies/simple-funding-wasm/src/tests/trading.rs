//! Entries, pairs, unwinds and what a read that fails does.

use bb_strategy_sdk::testing::d;
use bb_strategy_sdk::{Decimal, Intent, Side, StrategyAdapter, StrategyReport, VenueCall};

use super::harness::{Sim, settings};

fn ready(extra: &str) -> Sim {
    let mut sim = Sim::new(settings(extra), &["S1", "L1"]);
    sim.start().unwrap();
    sim
}

#[test]
fn it_enters_a_short_and_a_long_as_capped_iocs_through_the_universe_leg() {
    let mut sim = ready("");
    let intents = sim.tick();
    let places: Vec<_> = intents
        .iter()
        .filter_map(|i| match i {
            Intent::Request { leg, call: VenueCall::Place(o), .. } => Some((leg.as_str(), o)),
            _ => None,
        })
        .collect();
    assert_eq!(places.len(), 2, "{places:?}");
    assert!(places.iter().all(|(leg, _)| *leg == "universe"));
    let short = sim.orders().iter().find(|o| o.symbol == "S1").unwrap();
    let long = sim.orders().iter().find(|o| o.symbol == "L1").unwrap();
    assert_eq!((short.side, short.reduce_only, short.quantity), (Side::Sell, false, d("0.5")));
    // The long is sized to what the short filled in USD (49.925), at its own capped price.
    assert_eq!((long.side, long.reduce_only, long.quantity), (Side::Buy, false, d("0.4")));
    assert_eq!(short.price, d("99.95") * d("0.999"), "the touch less 10 bp");
    assert_eq!(long.price, d("100.05") * d("1.001"));
    assert!(bb_strategy_support::is_own(short.client_id.as_deref().unwrap()));
    assert_eq!((sim.held("S1"), sim.held("L1")), (d("-0.5"), d("0.4")));
}

#[test]
fn a_failed_position_read_trades_nothing() {
    let mut sim = ready("");
    sim.venue.positions_down = true;
    sim.tick();
    assert!(sim.orders().is_empty(), "an unreadable venue is not a flat one");
}

#[test]
fn unknown_equity_or_margin_means_no_entries() {
    let mut sim = ready("");
    sim.venue.equity = None;
    sim.tick();
    assert!(sim.orders().is_empty(), "equity unknown");
    sim.venue.equity = Some(d("10000"));
    sim.venue.margin = None;
    sim.advance(1);
    sim.tick();
    assert!(sim.orders().is_empty(), "margin ratio unknown");
}

#[test]
fn a_stale_mark_blocks_entries_and_a_host_stop_blocks_them_too() {
    let mut sim = ready("");
    // The tick helper feeds fresh data; skip it and let the marks age past stale_secs.
    sim.advance(60);
    sim.send(bb_strategy_sdk::Event::Tick);
    assert_eq!(sim.orders(), []);
    sim.market.path_stop = Some("risk gate: stopped".into());
    sim.tick();
    assert!(sim.orders().is_empty(), "the host refuses new orders: none are tried");
}

#[test]
fn a_rejected_first_leg_never_opens_the_sibling() {
    let mut sim = ready("");
    sim.venue.reject.insert("S1".into());
    sim.tick();
    assert!(sim.orders().iter().all(|o| o.symbol == "S1"), "{:?}", sim.orders());
    assert_eq!((sim.held("S1"), sim.held("L1")), (Decimal::ZERO, Decimal::ZERO));
}

#[test]
fn a_rejected_second_leg_unwinds_the_first_and_leaves_no_one_sided_book() {
    let mut sim = ready("");
    sim.venue.reject.insert("L1".into());
    for _ in 0..3 {
        sim.tick();
        sim.advance(70);
    }
    assert_eq!(sim.held("S1"), Decimal::ZERO, "the short is closed again");
    assert!(sim.orders().iter().any(|o| o.symbol == "S1" && o.reduce_only));
}

#[test]
fn a_partly_filled_first_leg_sizes_the_second_to_it() {
    let mut sim = ready("");
    sim.venue.partial = Some(("S1".into(), d("0.5")));
    sim.tick();
    let sold = -sim.held("S1");
    let bought = sim.held("L1");
    assert_eq!(sold, d("0.2"), "half of 0.5, to the step");
    assert_eq!(bought, d("0.1"), "the long is sized to the 19.97 USD the short filled");
}

#[test]
fn an_ambiguous_submit_that_shows_on_the_venue_is_treated_as_filled() {
    let mut sim = ready("");
    sim.venue.ambiguous.insert("S1".into());
    sim.tick();
    // Both legs are held: the short landed although its call errored, so the long was sized to it.
    assert_eq!((sim.held("S1"), sim.held("L1")), (d("-0.5"), d("0.4")));
}

#[test]
fn it_holds_a_pair_and_does_not_enter_again() {
    let mut sim = ready("");
    sim.tick();
    // The long filled 40 of its 50 USD target: one top-up (a 10% gap), then nothing more.
    sim.advance(61);
    sim.tick();
    sim.advance(61);
    sim.tick();
    let n = sim.orders().len();
    sim.advance(61);
    sim.tick();
    assert_eq!(sim.orders().len(), n, "the held legs are at their target: nothing to do");
    assert!(
        sim.orders().iter().all(|o| o.symbol != "S1" || o.quantity == d("0.5")),
        "the short is left alone"
    );
    assert_eq!(sim.strategy.report().headline(), "carrying across 2 markets");
}

#[test]
fn a_flipped_signal_exits_the_leg_reduce_only() {
    let mut sim = ready("");
    sim.tick();
    sim.venue.funding.insert("S1".into(), d("-0.0006"));
    sim.advance(700);
    sim.tick();
    let exit = sim.orders().iter().rev().find(|o| o.symbol == "S1").unwrap();
    assert!(exit.reduce_only && exit.side == Side::Buy, "{exit:?}");
    assert_eq!(sim.held("S1"), Decimal::ZERO);
}
