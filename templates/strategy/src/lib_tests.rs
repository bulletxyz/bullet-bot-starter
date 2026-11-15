use bb_strategy_sdk::testing::{Market, SimHost, book, d, fill_of, placed};
use bb_strategy_sdk::{Order, Trade};

use super::*;

const NOON: u64 = 1_794_398_400_000;

fn strategy() -> __TYPE__ {
    __TYPE__::from_settings(
        r#"{"spacing_bps": "20", "order_size": "0.01", "max_position": "0.02"}"#,
    )
    .unwrap()
}

fn sim() -> SimHost<__TYPE__> {
    SimHost::new(strategy(), Market::new("BTC-USD", "99", "101", NOON))
}

/// The venue's update that `order` ended with `status`.
fn ended(order: &NewOrder, status: OrderStatus) -> Event {
    Event::Order(Order {
        id: "venue-1".into(),
        client_id: order.client_id.clone(),
        symbol: order.symbol.clone(),
        side: order.side,
        order_type: order.order_type,
        price: order.price,
        quantity: order.quantity,
        filled_quantity: Decimal::ZERO,
        status,
    })
}

fn cancels(intents: &[Intent]) -> Vec<&CancelOrder> {
    intents.iter().filter_map(|i| if let Intent::Cancel(c) = i { Some(c) } else { None }).collect()
}

#[test]
fn its_name_is_its_folders() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).file_name().unwrap();
    assert_eq!(folder.to_str(), Some(<__TYPE__ as Describe>::NAME));
}

#[test]
fn it_quotes_both_sides_around_the_mid() {
    let mut sim = sim();
    let sent = sim.start().unwrap();
    let orders = placed(&sent);
    assert_eq!(orders.len(), 2);
    assert_eq!((orders[0].side, orders[0].price), (Side::Buy, d("99.8")));
    assert_eq!((orders[1].side, orders[1].price), (Side::Sell, d("100.2")));
}

#[test]
fn client_ids_begin_with_the_start_time_so_a_later_run_never_reuses_one() {
    let mut first = sim();
    let ids: Vec<_> =
        placed(&first.start().unwrap()).iter().map(|o| o.client_id.clone().unwrap()).collect();
    assert_eq!(ids, [format!("{NOON}-1"), format!("{NOON}-2")]);

    let later = NOON + 60_000;
    let mut second = SimHost::new(strategy(), Market::new("BTC-USD", "99", "101", later));
    let again = second.start().unwrap();
    assert_eq!(placed(&again)[0].client_id.as_deref(), Some(format!("{later}-1").as_str()));
}

#[test]
fn a_filled_side_is_placed_again_on_the_next_book() {
    let mut sim = sim();
    let sent = sim.start().unwrap();
    let buy = placed(&sent)[0].clone();
    sim.send(&fill_of(&buy));
    let again = sim.send(&Event::Book);
    assert_eq!(placed(&again).len(), 1);
    assert_eq!(placed(&again)[0].side, Side::Buy);
}

#[test]
fn a_partial_fill_keeps_the_side_until_the_rest_fills() {
    let mut sim = sim();
    let sent = sim.start().unwrap();
    let buy = placed(&sent)[0].clone();
    let Event::Fill(full) = fill_of(&buy) else { unreachable!() };
    let half = Trade { quantity: d("0.005"), ..full };

    sim.send(&Event::Fill(half.clone()));
    assert_eq!(placed(&sim.send(&Event::Book)), Vec::<&NewOrder>::new(), "the rest still rests");

    sim.send(&Event::Fill(half));
    assert_eq!(placed(&sim.send(&Event::Book)).len(), 1, "filled in full: placed again");
}

#[test]
fn an_order_the_mid_left_behind_is_cancelled_by_client_id_then_placed_again() {
    let mut sim = sim();
    let sent = sim.start().unwrap();
    let buy = placed(&sent)[0].clone();

    // The mid moves from 100 to 102, far more than a spacing.
    sim.market.book = Some(book("101", "103", "10"));
    let moved = sim.send(&Event::Book);
    let asked = cancels(&moved);
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[0].order_id, "", "by client id");
    assert_eq!(asked[0].client_id, buy.client_id);
    assert_eq!(cancels(&sim.send(&Event::Book)), Vec::<&CancelOrder>::new(), "asked once");

    // The venue confirms the buy's cancel: a new buy at the new mid.
    let replaced = sim.send(&ended(&buy, OrderStatus::Cancelled));
    let orders = placed(&replaced);
    assert_eq!(orders.len(), 1);
    assert_eq!((orders[0].side, orders[0].price), (Side::Buy, d("101.796")));
}

#[test]
fn it_places_no_order_that_would_take_the_position_past_max_position() {
    let mut market = Market::new("BTC-USD", "99", "101", NOON);
    market.position = d("0.02");
    let mut sim = SimHost::new(strategy(), market);
    let sent = sim.start().unwrap();
    let orders = placed(&sent);
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].side, Side::Sell, "only the side that brings it back");
}

#[test]
fn an_unknown_placement_holds_its_side_until_a_fresh_read_omits_it() {
    let mut sim = sim();
    let sent = sim.start().unwrap();
    let buy = placed(&sent)[0].clone();
    sim.send(&Event::Unknown { intent: Intent::Place(buy.clone()), error: "timeout".into() });
    assert_eq!(placed(&sim.send(&Event::Book)), Vec::<&NewOrder>::new(), "may rest: held");

    // A read that began too soon after the place decides nothing.
    let sell = placed(&sent)[1].clone();
    let listed = |o: &NewOrder| {
        let Event::Order(order) = ended(o, OrderStatus::Open) else { unreachable!() };
        order
    };
    sim.market.open_orders = Some(vec![listed(&sell)]);
    sim.market.read_started_ms = Some(NOON + 1_000);
    assert_eq!(placed(&sim.send(&Event::Tick)), Vec::<&NewOrder>::new());

    // A later one omits the buy (it never landed) and lists the sell: only the buy is placed.
    sim.market.read_started_ms = Some(NOON + LIST_LAG_MS);
    let again = sim.send(&Event::Tick);
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
    let bad = [
        r#"{"spacing_bps": "0", "order_size": "1", "max_position": "1"}"#,
        r#"{"spacing_bps": "20", "order_size": "1"}"#,
        r#"{"spacing_bps": "20", "order_size": "1", "max_position": "0.5"}"#,
    ];
    for settings in bad {
        assert!(__TYPE__::from_settings(settings).is_err(), "{settings}");
    }
}
