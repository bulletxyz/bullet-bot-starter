use bb_strategy_sdk::testing::{Market, SimHost, d, fill_of, placed};

use super::*;

const NOON: u64 = 1_794_398_400_000;

fn strategy() -> __TYPE__ {
    __TYPE__::from_settings(r#"{"spacing_bps": "20", "order_size": "0.01"}"#).unwrap()
}

#[test]
fn its_name_is_its_folders() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).file_name().unwrap();
    assert_eq!(folder.to_str(), Some(<__TYPE__ as Describe>::NAME));
}

#[test]
fn it_quotes_both_sides_around_the_mid() {
    let mut sim = SimHost::new(strategy(), Market::new("BTC-USD", "99", "101", NOON));
    let sent = sim.start().unwrap();
    let orders = placed(&sent);
    assert_eq!(orders.len(), 2);
    assert_eq!((orders[0].side, orders[0].price), (Side::Buy, d("99.8")));
    assert_eq!((orders[1].side, orders[1].price), (Side::Sell, d("100.2")));
}

#[test]
fn a_filled_side_is_placed_again_on_the_next_book() {
    let mut sim = SimHost::new(strategy(), Market::new("BTC-USD", "99", "101", NOON));
    let sent = sim.start().unwrap();
    let buy = placed(&sent)[0].clone();
    sim.send(&fill_of(&buy));
    let again = sim.send(&Event::Book);
    assert_eq!(placed(&again).len(), 1);
    assert_eq!(placed(&again)[0].side, Side::Buy);
}

#[test]
fn it_waits_while_orders_are_stopped_and_cancels_all_at_stop() {
    let mut market = Market::new("BTC-USD", "99", "101", NOON);
    market.path_stop = Some("daily loss stop".into());
    let mut sim = SimHost::new(strategy(), market);
    assert_eq!(sim.start().unwrap(), Vec::new());
    assert_eq!(sim.stop(), vec![Intent::CancelAll]);
}

#[test]
fn bad_settings_are_refused() {
    assert!(__TYPE__::from_settings(r#"{"spacing_bps": "0", "order_size": "1"}"#).is_err());
    assert!(__TYPE__::from_settings(r#"{"spacing_bps": "20"}"#).is_err());
}
