//! What the port does differently from the built-in (the rest is the parity fixtures' job).

use bb_strategy_sdk::testing::{Market, SimHost, placed};
use bb_strategy_sdk::{FromSettings, Intent, StrategyReport};
use bb_strategy_support::is_own;

use super::*;

const NOON: u64 = 1_794_398_400_000;

fn grid(extra: &str) -> GridStrategy {
    let json = format!(
        r#"{{"lower_price": "70", "upper_price": "80", "num_levels": 6, "order_size": "0.1"{extra}}}"#
    );
    GridStrategy::from_settings(&json).unwrap()
}

fn market(bid: &str, ask: &str) -> Market {
    Market::new("BTC-USD", bid, ask, NOON)
}

#[test]
fn the_start_places_resting_levels_under_the_bots_own_ids() {
    let mut sim = SimHost::new(grid(""), market("75.4", "75.6"));
    let sent = sim.start().unwrap();
    let orders = placed(&sent);
    assert_ne!(orders.len(), 0);
    assert!(orders.iter().all(|o| is_own(o.client_id.as_deref().unwrap()) && !o.reduce_only));
    assert!(sim.strategy.report().headline().contains("between 70 and 80"));
}

#[test]
fn a_grid_with_no_range_is_refused_when_loaded() {
    assert!(GridStrategy::from_settings("{}").is_err());
}

#[test]
fn past_the_hard_stop_it_cancels_its_orders_closes_reduce_only_and_stops() {
    let mut sim = SimHost::new(grid(""), market("75.4", "75.6"));
    sim.start().unwrap();
    sim.market.position = Decimal::new(2, 1);
    // The hard stop is half a range width below the range: 65.
    sim.market.book = Some(bb_strategy_sdk::testing::book("64.9", "65.1", "10"));
    let sent = sim.send(&Event::Book);
    assert_eq!(sent.first(), Some(&Intent::CancelAll), "{sent:?}");
    let close = placed(&sent);
    assert!(close.len() == 1 && close[0].reduce_only && close[0].side == Side::Sell, "{sent:?}");
    assert!(sim.strategy.report().headline().starts_with("stopped: hard stop"));
}

#[test]
fn the_stop_cancels_everything_it_placed() {
    let mut sim = SimHost::new(grid(""), market("75.4", "75.6"));
    sim.start().unwrap();
    assert_eq!(sim.stop(), vec![Intent::CancelAll]);
}
