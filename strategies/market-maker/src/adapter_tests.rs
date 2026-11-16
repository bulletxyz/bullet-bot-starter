//! The quotes: what a refresh sends, how each answer settles a side, and the pull while stopped.

#![expect(clippy::arithmetic_side_effects, reason = "test arithmetic")]

use bb_strategy_sdk::testing::{Market, d, outcome};
use bb_strategy_sdk::{
    AmendOrder, CancelOrder, Event, FromSettings, Intent, NewOrder, Order, OrderStatus, OrderType,
    Side, StrategyAdapter,
};
use serde_json::{Value, json};

use super::*;

pub(crate) const T0: u64 = 1_793_102_400_000;

/// The market maker on the bot's test base (10 bps a side, 0.1 a quote, max position 1,
/// post-only, a refresh every second past 1 bps of drift, the Bullet mid as fair value),
/// `overrides` on top, started on BTC-USD at 99/101.
pub(crate) struct Rig {
    pub mm: MarketMaker,
    pub m: Market,
    orders: u64,
}

impl Rig {
    pub fn new(overrides: &Value) -> Self {
        Self::at_position(overrides, "0")
    }

    pub fn at_position(overrides: &Value, position: &str) -> Self {
        let mut settings = json!({
            "symbol": "BTC-USD", "bid_spread_bps": "10", "ask_spread_bps": "10",
            "order_size": "0.1", "max_position": "1", "refresh_secs": 1,
            "refresh_threshold_bps": "1", "order_type": "post_only", "fair_value": "bullet",
        });
        for (k, v) in overrides.as_object().unwrap() {
            settings[k] = v.clone();
        }
        let mut mm = MarketMaker::from_settings(&settings.to_string()).unwrap();
        let mut m = Market::new("BTC-USD", "99", "101", T0);
        m.position = d(position);
        assert_eq!(mm.start(&m.view()).unwrap(), Vec::new(), "nothing at start");
        Self { mm, m, orders: 0 }
    }

    pub fn send(&mut self, event: &Event) -> Vec<Intent> {
        self.mm.on_event(event, &self.m.view())
    }

    pub fn book(&mut self, bid: &str, ask: &str) -> Vec<Intent> {
        self.m.book = Some(bb_strategy_sdk::testing::book(bid, ask, "10"));
        self.send(&Event::Book)
    }

    pub fn wait(&mut self, ms: u64) {
        self.m.now_ms += ms;
    }

    /// The venue takes every order intent of `intents`: their answers, and what they led to.
    pub fn take(&mut self, intents: &[Intent]) -> Vec<Intent> {
        let mut next = Vec::new();
        for intent in intents {
            let id = match intent {
                Intent::Place(_) | Intent::Amend(_) => {
                    self.orders += 1;
                    format!("v-{}", self.orders)
                }
                Intent::Cancel(c) => c.order_id.clone(),
                Intent::CancelOwn => String::new(),
                _ => continue,
            };
            next.extend(self.send(&outcome(intent, Ok(&id))));
        }
        next
    }

    pub fn refuse(&mut self, intent: &Intent, why: &str) -> Vec<Intent> {
        self.send(&outcome(intent, Err(why)))
    }

    /// The quote on `side` as the market maker holds it.
    pub fn slot(&self, side: Side) -> Option<&Slot> {
        self.mm.slots[side_index(side)].as_ref()
    }
}

pub(crate) fn place(side: Side, price: &str, qty: &str, cid: &str) -> Intent {
    Intent::Place(NewOrder {
        symbol: "BTC-USD".into(),
        side,
        order_type: OrderType::PostOnly,
        price: d(price),
        quantity: d(qty),
        client_id: Some(cid.into()),
        reduce_only: false,
        protect: None,
    })
}

fn cancel(oid: &str, cid: &str) -> CancelOrder {
    CancelOrder { symbol: "BTC-USD".into(), order_id: oid.into(), client_id: Some(cid.into()) }
}

fn cid_of(intent: &Intent) -> String {
    match intent {
        Intent::Place(o) | Intent::Amend(AmendOrder { new_order: o, .. }) => {
            o.client_id.clone().unwrap()
        }
        Intent::Cancel(c) => c.client_id.clone().unwrap(),
        other => panic!("no client id: {other:?}"),
    }
}

fn order(oid: &str, cid: &str, side: Side, status: OrderStatus) -> Order {
    Order {
        id: oid.into(),
        client_id: Some(cid.into()),
        symbol: "BTC-USD".into(),
        side,
        order_type: OrderType::PostOnly,
        price: d("100"),
        quantity: d("0.1"),
        filled_quantity: d("0"),
        status,
    }
}

/// The first book quotes both sides at the fair value ± the spreads; the answers make them the
/// slots, with the venue's ids.
#[test]
fn the_first_book_places_both_sides_and_their_answers_make_the_slots() {
    let mut r = Rig::new(&json!({}));
    let sent = r.book("99", "101");
    let (bid, ask) = (cid_of(&sent[0]), cid_of(&sent[1]));
    assert_eq!(
        sent,
        [place(Side::Buy, "99.900", "0.1", &bid), place(Side::Sell, "100.100", "0.1", &ask)]
    );
    assert!(r.take(&sent).is_empty());
    assert_eq!(r.slot(Side::Buy).unwrap().order_id.as_deref(), Some("v-1"));
    assert_eq!(r.slot(Side::Sell).unwrap().order_id.as_deref(), Some("v-2"));
}

/// Under the drift threshold nothing moves; past it both quotes are repriced, each in one amend
/// of the resting order.
#[test]
fn a_small_drift_keeps_the_quotes_and_a_large_one_amends_both() {
    let mut r = Rig::new(&json!({ "refresh_threshold_bps": "5" }));
    let sent = r.book("99", "101");
    let (bid, ask) = (cid_of(&sent[0]), cid_of(&sent[1]));
    r.take(&sent);
    r.wait(2_000);
    assert!(r.book("99.02", "101.02").is_empty(), "2 bps: under the threshold");
    r.wait(2_000);
    let sent = r.book("99.1", "101.1");
    let [Intent::Amend(b), Intent::Amend(a)] = sent.as_slice() else { panic!("{sent:?}") };
    assert_eq!((b.cancel.clone(), a.cancel.clone()), (cancel("v-1", &bid), cancel("v-2", &ask)));
    assert_eq!((b.new_order.price, a.new_order.price), (d("99.9999"), d("100.2001")));
    r.take(&sent);
    assert_eq!(r.slot(Side::Buy).unwrap().price, d("99.9999"));
}

/// A refresh waits `refresh_secs` from the last one.
#[test]
fn a_refresh_waits_its_cadence() {
    let mut r = Rig::new(&json!({ "refresh_secs": 5 }));
    let sent = r.book("99", "101");
    r.take(&sent);
    r.wait(1_000);
    assert!(r.book("98", "100").is_empty());
    r.wait(4_000);
    assert_eq!(r.book("98", "100").len(), 2);
}

/// At the cap the side that would grow the position is not quoted (characterised on the bot's
/// rig: `characterise_max_position_suppresses_growing_side_long` / `_short`); at the cap itself
/// only the exit is.
#[test]
fn the_side_that_would_grow_past_the_cap_is_not_quoted() {
    for (position, side) in [("0.95", Side::Sell), ("-0.95", Side::Buy), ("1", Side::Sell)] {
        let mut r = Rig::at_position(&json!({}), position);
        let sent = r.book("99", "101");
        let sides: Vec<_> = sent
            .iter()
            .map(|i| match i {
                Intent::Place(o) => o.side,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(sides, [side], "at {position}");
    }
}

/// Once a fill takes the position to the cap, the next refresh cancels the quote on the side that
/// would grow it.
#[test]
fn a_quote_the_position_has_outgrown_is_cancelled() {
    let mut r = Rig::new(&json!({}));
    let sent = r.book("99", "101");
    let bid = cid_of(&sent[0]);
    r.take(&sent);
    r.m.position = d("0.95");
    r.wait(1_000);
    let sent = r.book("99", "101");
    assert_eq!(sent, [Intent::Cancel(cancel("v-1", &bid))]);
    r.take(&sent);
    assert!(r.slot(Side::Buy).is_none());
}

/// Near the cap the replacement would not pass with the old bid counted as filled: the bid is
/// cancelled first and its replacement placed on the cancel's answer, re-capped at the position
/// then. The ask, which shrinks the position, is one amend. Short is the mirror.
#[test]
fn near_the_cap_a_reprice_cancels_first_and_places_on_the_answer() {
    for (position, near, far) in [("0.85", Side::Buy, Side::Sell), ("-0.85", Side::Sell, Side::Buy)]
    {
        let mut r = Rig::at_position(&json!({}), position);
        let sent = r.book("99", "101");
        r.take(&sent);
        let old = r.slot(near).unwrap().clone();
        r.wait(1_000);
        let sent = r.book("99.5", "101.5");
        let cancels: Vec<_> = sent.iter().filter(|i| matches!(i, Intent::Cancel(_))).collect();
        assert_eq!(cancels, [&Intent::Cancel(old.cancel("BTC-USD"))], "{sent:?}");
        assert!(sent.iter().any(|i| matches!(i, Intent::Amend(a) if a.new_order.side == far)));
        let after = r.take(&sent);
        let [Intent::Place(new)] = after.as_slice() else { panic!("{after:?}") };
        assert_eq!((new.side, new.quantity), (near, d("0.1")));
        r.take(&after);
        assert_eq!(r.slot(near).unwrap().client_id, new.client_id.clone().unwrap());
    }
}

/// A fill while the cancel is out moves the position to the cap: the replacement is dropped.
#[test]
fn a_fill_during_a_two_step_reprice_recaps_its_replacement() {
    let mut r = Rig::at_position(&json!({}), "0.85");
    let sent = r.book("99", "101");
    r.take(&sent);
    r.wait(1_000);
    let sent = r.book("99.5", "101.5");
    r.m.position = d("0.95");
    assert!(r.take(&sent).is_empty(), "the bid would grow the position past the cap");
}

/// A stop arriving while the cancel is out: no replacement.
#[test]
fn a_stop_during_a_two_step_reprice_places_no_replacement() {
    let mut r = Rig::at_position(&json!({}), "0.85");
    let sent = r.book("99", "101");
    r.take(&sent);
    r.wait(1_000);
    let sent = r.book("99.5", "101.5");
    r.m.path_stop = Some("stale price".into());
    assert!(r.take(&sent).is_empty());
}

/// A place whose outcome is unknown holds its side: it is not placed again, and its reprice
/// waits for a cancel (by client id, the venue id unknown). A read that started the settle margin
/// later and does not list it frees the side.
#[test]
fn an_unknown_place_holds_its_side_until_a_read_settles_it() {
    let mut r = Rig::new(&json!({}));
    let sent = r.book("99", "101");
    let bid = cid_of(&sent[0]);
    r.send(&Event::Unknown { intent: sent[0].clone(), error: "timeout".into() });
    r.take(&sent[1..]);
    assert!(r.slot(Side::Buy).unwrap().unknown);
    assert!(r.mm.wants_venue(r.m.now_ms, false), "a read settles it");
    r.wait(1_000);
    let sent = r.book("99.5", "101.5");
    assert!(sent.contains(&Intent::Cancel(cancel("", &bid))), "{sent:?}");
    assert!(!sent.iter().any(|i| matches!(i, Intent::Place(o) if o.side == Side::Buy)));
    r.refuse(&sent[0], "not found");
    r.take(&sent[1..]);

    // A read within the margin keeps it; one past it frees the side.
    r.wait(500);
    r.m.open_orders = Some(Vec::new());
    r.m.read_started_ms = Some(r.m.now_ms);
    r.send(&Event::Tick);
    assert!(r.slot(Side::Buy).is_some());
    r.wait(2_000);
    r.m.read_started_ms = Some(r.m.now_ms);
    let sent = r.send(&Event::Tick);
    assert!(sent.iter().any(|i| matches!(i, Intent::Place(o) if o.side == Side::Buy)), "{sent:?}");
}

/// An unknown place the read lists rests after all: it takes the venue's id.
#[test]
fn a_read_that_lists_an_unknown_place_adopts_it() {
    let mut r = Rig::new(&json!({}));
    let sent = r.book("99", "101");
    let bid = cid_of(&sent[0]);
    r.send(&Event::Unknown { intent: sent[0].clone(), error: "timeout".into() });
    r.wait(3_000);
    r.m.open_orders = Some(vec![order("v-9", &bid, Side::Buy, OrderStatus::Open)]);
    r.m.read_started_ms = Some(r.m.now_ms);
    r.send(&Event::Tick);
    let slot = r.slot(Side::Buy).unwrap();
    assert_eq!((slot.order_id.as_deref(), slot.unknown), (Some("v-9"), false));
}

/// The amend's new order placed but its cancel failed: the old order may still rest beside it.
/// It is a stray: the next refresh cancels it and places nothing until it is gone.
#[test]
fn a_failed_cancel_beside_its_amend_leaves_a_stray_that_blocks_placing() {
    let mut r = Rig::new(&json!({}));
    let sent = r.book("99", "101");
    r.take(&sent);
    let old = r.slot(Side::Buy).unwrap().cancel("BTC-USD");
    r.wait(1_000);
    let sent = r.book("99.5", "101.5");
    r.take(&sent);
    r.refuse(&Intent::Cancel(old.clone()), "rejected");
    assert_eq!(r.mm.strays.len(), 1);
    // A fill took the ask: its side is empty, but nothing is placed while the stray may rest.
    let ask = r.slot(Side::Sell).unwrap().clone();
    r.send(&Event::Order(order(
        ask.order_id.as_deref().unwrap(),
        &ask.client_id,
        Side::Sell,
        OrderStatus::Filled,
    )));
    r.wait(1_000);
    let sent = r.book("99.5", "101.5");
    assert_eq!(sent, [Intent::Cancel(old)], "the stray first, no place");
    r.take(&sent);
    r.wait(1_000);
    let sent = r.book("99.5", "101.5");
    assert!(matches!(sent.as_slice(), [Intent::Place(o)] if o.side == Side::Sell), "{sent:?}");
}

/// A stray that filled before its cancel landed is gone: its terminal update clears it.
#[test]
fn a_stray_that_filled_blocks_nothing() {
    let mut r = Rig::new(&json!({}));
    let sent = r.book("99", "101");
    r.take(&sent);
    let old = r.slot(Side::Buy).unwrap().clone();
    r.wait(1_000);
    let sent = r.book("99.5", "101.5");
    r.take(&sent);
    r.refuse(&Intent::Cancel(old.cancel("BTC-USD")), "rejected");
    let oid = old.order_id.clone().unwrap();
    r.send(&Event::Order(order(&oid, &old.client_id, Side::Buy, OrderStatus::Filled)));
    assert!(r.mm.strays.is_empty());
}

/// A refused amend may have left its old order resting (its cancel may not have gone): the old
/// quote is a stray, cancelled before anything is placed.
#[test]
fn a_refused_amend_leaves_the_old_quote_a_stray() {
    let mut r = Rig::new(&json!({}));
    let sent = r.book("99", "101");
    r.take(&sent);
    let old = r.slot(Side::Buy).unwrap().cancel("BTC-USD");
    r.wait(1_000);
    let sent = r.book("99.5", "101.5");
    r.refuse(&sent[0], "post only would cross");
    assert!(r.slot(Side::Buy).is_none());
    assert_eq!(r.mm.strays.iter().map(|s| s.cancel.clone()).collect::<Vec<_>>(), [old]);
}

/// A call that failed as a whole was never sent: an amend's or a cancel's old quote is still the
/// slot, and a place leaves none.
#[test]
fn a_call_that_failed_as_a_whole_keeps_what_it_asked_about() {
    let mut r = Rig::new(&json!({}));
    let sent = r.book("99", "101");
    r.send(&Event::Failed { intent: sent[0].clone(), error: "down".into() });
    r.take(&sent[1..]);
    assert!(r.slot(Side::Buy).is_none());
    let old = r.slot(Side::Sell).unwrap().clone();
    r.wait(1_000);
    let sent = r.book("99.5", "101.5");
    let amend = sent.iter().find(|i| matches!(i, Intent::Amend(_))).unwrap().clone();
    r.send(&Event::Failed { intent: amend, error: "down".into() });
    assert_eq!(r.slot(Side::Sell), Some(&old));
}

/// A read drops a quote the venue no longer lists, but only one answered at least the settle
/// margin before the read started.
#[test]
fn a_read_drops_a_quote_gone_from_the_venue_after_the_settle_margin() {
    let mut r = Rig::new(&json!({}));
    let sent = r.book("99", "101");
    r.take(&sent);
    r.m.open_orders = Some(Vec::new());
    r.m.read_started_ms = Some(r.m.now_ms + 1_000);
    r.wait(1_000);
    r.send(&Event::Tick);
    assert!(r.slot(Side::Buy).is_some(), "within the margin");
    r.wait(2_000);
    r.m.read_started_ms = Some(r.m.now_ms);
    let bid = r.slot(Side::Buy).unwrap().clone();
    r.m.open_orders = Some(vec![order("v-1", &bid.client_id, Side::Buy, OrderStatus::Open)]);
    r.send(&Event::Tick);
    assert!(r.slot(Side::Buy).is_some() && r.slot(Side::Sell).is_none());
}

/// While the host stops new orders the quotes are pulled once (the market maker's own orders),
/// and the first book after the stop lifts quotes at once.
#[test]
fn a_stop_pulls_once_and_the_first_book_after_it_quotes_at_once() {
    let mut r = Rig::new(&json!({ "refresh_secs": 60 }));
    let sent = r.book("99", "101");
    r.take(&sent);
    r.m.path_stop = Some("the daily loss stop".into());
    assert_eq!(r.book("99", "101"), [Intent::CancelOwn]);
    assert!(r.book("99", "101").is_empty(), "asked once");
    assert!(r.take(&[Intent::CancelOwn]).is_empty());
    assert!(r.slot(Side::Buy).is_none() && r.slot(Side::Sell).is_none());
    assert!(r.book("99", "101").is_empty(), "nothing rests: nothing to pull");
    r.m.path_stop = None;
    assert_eq!(r.book("99", "101").len(), 2, "within the cadence, but after a pull");
}

/// A pull that failed is asked again, at most every ten seconds, until one lands.
#[test]
fn a_failed_pull_is_asked_again_every_ten_seconds() {
    let mut r = Rig::new(&json!({}));
    let sent = r.book("99", "101");
    r.take(&sent);
    r.m.venue_halt = Some("stale oracle".into());
    let pull = r.send(&Event::Tick);
    r.refuse(&pull[0], "down");
    r.wait(5_000);
    assert!(r.send(&Event::Tick).is_empty());
    assert!(!r.mm.may_keep(), "a failing pull keeps nothing");
    r.wait(5_000);
    assert_eq!(r.send(&Event::Tick), [Intent::CancelOwn]);
}

/// A book with no two sides is no price to quote around: what rests is pulled.
#[test]
fn a_book_without_two_sides_pulls_the_quotes() {
    let mut r = Rig::new(&json!({}));
    let sent = r.book("99", "101");
    r.take(&sent);
    r.m.book.as_mut().unwrap().asks.clear();
    assert_eq!(r.send(&Event::Book), [Intent::CancelOwn]);
}

/// The host's cancel of every order (its wind-down, its pause) empties the slots, and the next
/// refresh is due at once.
#[test]
fn the_hosts_cancel_of_every_order_empties_the_slots() {
    let mut r = Rig::new(&json!({ "refresh_secs": 60 }));
    let sent = r.book("99", "101");
    r.take(&sent);
    r.mm.orders_cancelled();
    assert!(r.slot(Side::Buy).is_none());
    assert_eq!(r.book("99", "101").len(), 2);
}

/// The venue's order ids come from its updates as well as from the answers.
#[test]
fn an_open_update_gives_an_unknown_quote_its_venue_id() {
    let mut r = Rig::new(&json!({}));
    let sent = r.book("99", "101");
    let bid = cid_of(&sent[0]);
    r.send(&Event::Unknown { intent: sent[0].clone(), error: "timeout".into() });
    r.send(&Event::Order(order("v-7", &bid, Side::Buy, OrderStatus::Open)));
    assert_eq!(r.slot(Side::Buy).unwrap().order_id.as_deref(), Some("v-7"));
}

/// The report: the live params, the fair value of the last event, and the time spent with a quote
/// up and no stop (a gap between events adds at most a minute).
#[test]
fn the_report_shows_the_fair_value_and_the_time_quoting() {
    let mut r = Rig::new(&json!({ "refresh_secs": 600 }));
    let report = r.mm.report();
    assert_eq!(report.fair_source, Some(crate::FairValue::Bullet), "the start's book");
    let sent = r.book("99", "101");
    r.take(&sent);
    r.wait(30_000);
    r.send(&Event::Tick);
    r.wait(120_000);
    r.send(&Event::Tick);
    let report = r.mm.report();
    assert_eq!((report.fair_value, report.paper_quoting_secs), (Some(d("100")), 90));
    assert_eq!(report.params.order_size, d("0.1"));
    assert_eq!(report.extra_params.len(), PARAMS.len());
}

/// Run as the host runs it: every intent answered, fills moving the position, and the skew moving
/// both quotes down once long.
#[test]
fn on_the_simulated_host_it_quotes_fills_and_skews() {
    use bb_strategy_sdk::testing::{SimHost, fill_of, placed};
    let settings = json!({
        "symbol": "BTC-USD", "bid_spread_bps": "10", "ask_spread_bps": "10", "order_size": "0.5",
        "max_position": "1", "refresh_secs": 1, "skew_bps": "10", "fair_value": "bullet",
    });
    let mm = MarketMaker::from_settings(&settings.to_string()).unwrap();
    let mut host = SimHost::new(mm, Market::new("BTC-USD", "99", "101", T0));
    host.start().unwrap();
    let first = host.send(&Event::Book);
    let orders: Vec<NewOrder> = placed(&first).into_iter().cloned().collect();
    assert_eq!(orders.len(), 2);
    host.send(&fill_of(&orders[0]));
    host.send(&Event::Order(Order {
        id: "sim-1".into(),
        client_id: orders[0].client_id.clone(),
        filled_quantity: d("0.5"),
        status: OrderStatus::Filled,
        ..order("sim-1", "", Side::Buy, OrderStatus::Filled)
    }));
    host.market.now_ms += 1_000;
    let next = host.send(&Event::Book);
    // Long 0.5 of 1 with 10 bps of skew: 5 bps lower; the bid is placed again, the ask amended.
    let bid = placed(&next).into_iter().find(|o| o.side == Side::Buy).cloned().unwrap();
    assert_eq!(bid.price, d("99.85"));
    assert!(next.iter().any(|i| matches!(i, Intent::Amend(a) if a.new_order.price == d("100.05"))));
}
