//! What the port does differently from the built-in (the rest is the parity fixtures' job).

#![expect(
    clippy::assert_is_empty,
    reason = "an empty call is the claim, asserted where it is made"
)]

use bb_strategy_sdk::LegNeed;
use bb_strategy_sdk::testing::{Market, SimHost, book, d};
use bb_strategy_sdk::{
    Answer, CallError, Describe, FieldValue, FromSettings, LegView, OrderResult, StrategyReport,
};
use bb_strategy_support::is_own;

use super::*;
use crate::AvellanedaStoikov as Strategy;

const SYM: &str = "BTC-USD";
const T0: u64 = 1_794_398_400_000;

fn strategy(extra: &str) -> Strategy {
    let mut json: serde_json::Value = serde_json::from_str(
        r#"{"order_size": "1", "max_position": "5", "gamma": "1", "kappa": "1.5",
            "order_levels": 2, "min_refresh_interval_ms": 0}"#,
    )
    .unwrap();
    let extra: serde_json::Value =
        serde_json::from_str(&format!("{{{}}}", extra.trim_start_matches(','))).unwrap();
    json.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    Strategy::from_settings(&json.to_string()).unwrap()
}

fn market() -> Market {
    Market::new(SYM, "99.95", "100.05", T0)
}

fn quote_book(sim: &mut SimHost<Strategy>, bid: &str, ask: &str, at: u64) -> Vec<Intent> {
    sim.market.now_ms = at;
    sim.market.book = Some(book(bid, ask, "10"));
    sim.send(&Event::Book)
}

/// A started strategy with its first ladder out (the third sample).
fn quoting() -> SimHost<Strategy> {
    let mut sim = SimHost::new(strategy(""), market());
    sim.start().unwrap();
    quote_book(&mut sim, "100.05", "100.15", T0 + 1);
    let sent = quote_book(&mut sim, "100.20", "100.30", T0 + 2);
    assert_eq!(placed(&sent).len(), 4, "{sent:?}");
    sim
}

fn placed(intents: &[Intent]) -> Vec<&NewOrder> {
    bb_strategy_sdk::testing::placed(intents)
}

#[test]
fn the_start_cancels_everything_and_the_stop_does_too() {
    let mut sim = SimHost::new(strategy(""), market());
    assert_eq!(sim.start().unwrap(), vec![Intent::CancelAll]);
    assert_eq!(sim.stop(), vec![Intent::CancelAll]);
}

#[test]
fn a_start_needs_a_book_and_the_market_the_settings_name() {
    let mut m = market();
    m.book = None;
    let e = strategy("").start(&m.view()).unwrap_err();
    assert!(e.0.contains("No orderbook data"), "{e}");
    assert!(strategy(r#", "symbol": "ETH-USD""#).start(&market().view()).is_err());
    assert!(strategy(r#", "symbol": "BTC-USD""#).start(&market().view()).is_ok());
    // Fair-value mode needs no local mid.
    let mut s = strategy(r#", "reference_exchange": "binance", "reference_symbol": "x""#);
    assert!(s.start(&m.view()).is_ok());
}

#[test]
fn the_fee_floor_stops_the_start() {
    let mut s = strategy(r#", "fees": {"maker_bps": "4"}"#);
    assert!(s.start(&market().view()).is_err());
}

#[test]
fn quotes_carry_the_bots_own_client_ids_and_both_sides_of_every_level() {
    let sim = quoting();
    let places: Vec<&NewOrder> = placed(&sim.sent);
    assert!(places.iter().all(|o| is_own(o.client_id.as_deref().unwrap())));
    assert_eq!(places.iter().filter(|o| o.side == Side::Buy).count(), 2);
    assert_eq!(places.iter().filter(|o| o.side == Side::Sell).count(), 2);
    assert!(places.iter().all(|o| o.order_type == OrderType::PostOnly && !o.reduce_only));
    // The first ladder asks for the venue's open orders once.
    let reads = sim.sent.iter().filter(|i| matches!(i, Intent::Request { .. })).count();
    assert_eq!(reads, 1);
}

#[test]
fn an_unchanged_ladder_sends_nothing() {
    let mut sim = quoting();
    assert!(quote_book(&mut sim, "100.20", "100.30", T0 + 3).is_empty());
    sim.market.now_ms = T0 + 60_000;
    assert!(sim.send(&Event::Tick).is_empty());
}

#[test]
fn time_is_the_views_the_throttle_and_the_forced_refresh_read_it() {
    let mut sim = SimHost::new(strategy(r#", "min_refresh_interval_ms": 100"#), market());
    sim.start().unwrap();
    quote_book(&mut sim, "100.05", "100.15", T0 + 1);
    assert!(!quote_book(&mut sim, "100.20", "100.30", T0 + 2).is_empty());
    // 50 ms on: throttled, whatever the move. 150 ms on: amended.
    assert!(quote_book(&mut sim, "101.40", "101.50", T0 + 52).is_empty());
    assert!(!quote_book(&mut sim, "101.40", "101.50", T0 + 152).is_empty());
}

#[test]
fn the_volatility_window_is_in_the_views_milliseconds() {
    let mut sim = SimHost::new(strategy(r#", "vol_window_secs": 1"#), market());
    sim.start().unwrap();
    quote_book(&mut sim, "100.05", "100.15", T0 + 1);
    // A sample exactly a window old stays: three samples, so the first ladder goes out.
    assert!(!quote_book(&mut sim, "100.20", "100.30", T0 + 1_000).is_empty());
    // Two seconds on only the newest is inside the window: no sigma, no requote.
    assert!(quote_book(&mut sim, "101.40", "101.50", T0 + 3_100).is_empty());
}

#[test]
fn a_refused_place_drops_its_slot_so_the_next_requote_places_it_again() {
    let mut sim = SimHost::new(strategy(""), market());
    sim.refuse = |o| (o.side == Side::Sell).then_some("no");
    sim.start().unwrap();
    quote_book(&mut sim, "100.05", "100.15", T0 + 1);
    quote_book(&mut sim, "100.20", "100.30", T0 + 2);
    assert_eq!(sim.strategy.report().quotes, 2, "only the bids rest");
    sim.refuse = |_| None;
    let again = quote_book(&mut sim, "100.20", "100.30", T0 + 3);
    assert_eq!(placed(&again).iter().filter(|o| o.side == Side::Sell).count(), 2);
}

#[test]
fn a_refused_cancel_puts_the_slot_back_and_a_refused_amend_restores_the_old_one() {
    let mut sim = quoting();
    // A cancel (the cap turns the buy side off) is refused: the slot is tracked again.
    let mut s = strategy(r#", "max_position": "1""#);
    s.start(&market().view()).unwrap();
    let cancel =
        CancelOrder { symbol: SYM.into(), order_id: "o1".into(), client_id: Some("7".into()) };
    s.slots.push(QuoteSlot {
        side: Side::Buy,
        level: 0,
        price: d("99"),
        size: d("1"),
        client_id: Some("7".into()),
        order_id: Some("o1".into()),
    });
    let slot = s.slots.remove(0);
    s.cancelling.push(slot);
    let refused = OrderResult {
        order_id: None,
        client_id: Some("7".into()),
        success: false,
        error: Some("no".into()),
    };
    s.on_outcome(&Intent::Cancel(cancel), &refused);
    assert_eq!(s.slots.len(), 1, "tracking kept to retry");

    // An amend refused: the old slot is back, the new one gone.
    let before = sim.strategy.slots.clone();
    // Not through the sim: its outcomes would answer the amends before the test refuses them.
    sim.market.now_ms = T0 + 3;
    sim.market.book = Some(book("101.40", "101.50", "10"));
    let sent = sim.strategy.on_event(&Event::Book, &sim.market.view());
    let amends: Vec<&AmendOrder> =
        sent.iter().filter_map(|i| if let Intent::Amend(a) = i { Some(a) } else { None }).collect();
    assert_eq!(amends.len(), 4);
    for a in &amends {
        let refused = OrderResult {
            order_id: None,
            client_id: a.new_order.client_id.clone(),
            success: false,
            error: Some("no".into()),
        };
        sim.strategy.on_outcome(&Intent::Amend((*a).clone()), &refused);
    }
    let mut after = sim.strategy.slots.clone();
    let mut want = before;
    let key = |s: &QuoteSlot| (s.side as u8, s.level);
    after.sort_by_key(key);
    want.sort_by_key(key);
    assert_eq!(after, want);
}

#[test]
fn a_fill_clears_its_slot_and_forces_the_next_tick_to_requote() {
    let mut sim = quoting();
    let buy = placed(&sim.sent).into_iter().find(|o| o.side == Side::Buy).unwrap().clone();
    let quotes = sim.strategy.report().quotes;
    sim.market.now_ms = T0 + 10;
    assert!(sim.send(&bb_strategy_sdk::testing::fill_of(&buy)).is_empty());
    assert_eq!(sim.strategy.report().quotes, quotes - 1);
    assert_eq!(sim.strategy.report().net_position, d("1"));
    sim.market.now_ms = T0 + 11;
    let sent = sim.send(&Event::Tick);
    assert_eq!(placed(&sent).len(), 1, "the level that filled is placed again: {sent:?}");
}

#[test]
fn an_unknown_outcome_keeps_the_slot_and_asks_for_a_reconcile() {
    let mut sim = quoting();
    let place = placed(&sim.sent)[0].clone();
    let quotes = sim.strategy.report().quotes;
    sim.market.now_ms = T0 + 5;
    let held = Event::Unknown { intent: Intent::Place(place), error: "timeout".into() };
    assert!(sim.send(&held).is_empty());
    assert_eq!(sim.strategy.report().quotes, quotes, "held, not placed again");
}

#[test]
fn the_reconcile_cancels_orphans_and_drops_phantoms_when_the_read_answers() {
    let mut sim = quoting();
    let id = sim
        .sent
        .iter()
        .find_map(|i| if let Intent::Request { id, .. } = i { Some(*id) } else { None })
        .unwrap();
    let live = |oid: &str, cid: Option<&str>, price: &str| Order {
        id: oid.into(),
        client_id: cid.map(Into::into),
        symbol: SYM.into(),
        side: Side::Buy,
        order_type: OrderType::PostOnly,
        price: d(price),
        quantity: d("1"),
        filled_quantity: Decimal::ZERO,
        status: OrderStatus::Open,
    };
    // Only the first quote is live; a stray order is too.
    let first = sim.strategy.slots[0].clone();
    let open = vec![
        live(first.order_id.as_deref().unwrap(), first.client_id.as_deref(), "99"),
        live("stray", Some("777"), "90"),
    ];
    sim.market.now_ms = T0 + 20;
    let sent = sim.send(&Event::Answer { id, reply: Ok(Answer::OpenOrders(open)) });
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(matches!(&sent[0], Intent::Cancel(c) if c.order_id == "stray"));
    assert_eq!(sim.strategy.report().quotes, 1, "the three the venue lacks are dropped");
    // An answer for another request, or a failed one, changes nothing.
    assert!(sim.send(&Event::Answer { id: id + 9, reply: Ok(Answer::Done) }).is_empty());
    let failed = CallError { message: "x".into(), never_sent: true, retryable: true };
    assert!(sim.send(&Event::Answer { id, reply: Err(failed) }).is_empty());
}

#[test]
fn at_most_64_orphans_are_cancelled_in_one_answer() {
    let mut s = strategy("");
    s.start(&market().view()).unwrap();
    let open: Vec<Order> = (0..100)
        .map(|n| Order {
            id: format!("x{n}"),
            client_id: None,
            symbol: SYM.into(),
            side: Side::Sell,
            order_type: OrderType::Limit,
            price: d("120"),
            quantity: d("1"),
            filled_quantity: Decimal::ZERO,
            status: OrderStatus::Open,
        })
        .collect();
    assert_eq!(s.reconcile_with(&open).len(), MAX_INTENTS);
}

#[test]
fn a_full_ladder_stays_inside_the_intent_cap() {
    let mut sim = SimHost::new(
        strategy(&format!(r#", "order_levels": {}"#, crate::config::MAX_LEVELS)),
        market(),
    );
    sim.start().unwrap();
    quote_book(&mut sim, "100.05", "100.15", T0 + 1);
    let sent = quote_book(&mut sim, "100.20", "100.30", T0 + 2);
    assert!(sent.len() <= MAX_INTENTS, "{}", sent.len());
    assert_eq!(placed(&sent).len(), 2 * crate::config::MAX_LEVELS);
}

fn reference_sim() -> SimHost<Strategy> {
    let s = strategy(
        r#", "reference_exchange": "binance", "reference_symbol": "btcusdt",
            "reference_stale_secs": 1"#,
    );
    SimHost::new(s, market())
}

fn reference_book(sim: &mut SimHost<Strategy>, bid: &str, ask: &str, at: u64) -> Vec<Intent> {
    sim.market.now_ms = at;
    let mut leg = LegView::new(REFERENCE_LEG, "binance", vec!["BTCUSDT".into()]);
    leg.books.insert("BTCUSDT".into(), book(bid, ask, "10"));
    sim.market.legs = vec![leg];
    sim.send(&Event::Leg {
        leg: REFERENCE_LEG.into(),
        symbol: "BTCUSDT".into(),
        event: Box::new(Event::Book),
    })
}

#[test]
fn fair_value_mode_prices_off_the_reference_leg_and_stops_when_it_is_stale() {
    let mut sim = reference_sim();
    sim.start().unwrap();
    // The local book alone never quotes.
    assert!(quote_book(&mut sim, "100.05", "100.15", T0 + 1).is_empty());
    reference_book(&mut sim, "100.00", "100.10", T0 + 2);
    reference_book(&mut sim, "100.10", "100.20", T0 + 3);
    assert!(!reference_book(&mut sim, "100.20", "100.30", T0 + 4).is_empty());
    // The reference is older than a second: the local book changes nothing.
    assert!(quote_book(&mut sim, "98.00", "98.10", T0 + 2_500).is_empty());
    assert!(sim.strategy.report().reference_stale);
    // Another leg, or another symbol, is not the reference.
    sim.market.now_ms = T0 + 2_600;
    let other =
        Event::Leg { leg: "hedge".into(), symbol: "BTCUSDT".into(), event: Box::new(Event::Book) };
    assert!(sim.send(&other).is_empty());
    assert_eq!(sim.strategy.report().mode, "fair-value");
}

#[test]
fn the_descriptor_declares_the_reference_leg_and_the_schema() {
    let d = Strategy::describe();
    assert_eq!(d.legs, vec![LegNeed::new(REFERENCE_LEG, "binance")]);
    assert_eq!(<Strategy as Describe>::NAME, "avellaneda-stoikov-wasm");
    assert!(d.settings.iter().any(|s| s.path == "gamma" && s.required));
    assert!(d.settings.iter().any(|s| s.path == "fees.maker_bps"));
}

#[test]
fn the_report_fits_the_bots_limits() {
    let mut sim = quoting();
    let report = sim.strategy.report();
    assert!(report.headline().contains("4 quotes up"), "{}", report.headline());
    let fields = report.fields();
    assert!(fields.len() <= 128 && fields.iter().all(|f| f.name.len() <= 64));
    assert!(fields.iter().any(|f| f.name == "sigma" && matches!(f.value, FieldValue::Number(_))));
    // Before the third sample it says it waits.
    sim.strategy = strategy("");
    sim.strategy.start(&market().view()).unwrap();
    assert!(sim.strategy.report().headline().contains("waiting for price samples"));
}
