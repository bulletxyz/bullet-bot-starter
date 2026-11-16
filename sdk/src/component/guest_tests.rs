//! The component glue, run natively: what crosses the wire comes back as the strategy saw it.

use std::sync::Mutex;

use rust_decimal::Decimal;

use super::*;
use crate::host::calls::{Answer, CallError, ReadQuery, VenueCall};
use crate::host::legs::LegView;
use crate::strategy::adapter::{
    Event, FieldValue, Intent, MarketFacts, PaperInputs, ReportField, View,
};
use crate::types::{
    AccountEquity, AmendOrder, CancelOrder, CancelResult, FundingSample, NewOrder, Order,
    OrderBook, OrderResult, OrderStatus, OrderType, Position, Side, Trade,
};

#[path = "guest_carry_tests.rs"]
mod carry_tests;
#[path = "guest_warm_up_tests.rs"]
mod warm_up_tests;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn order(price: &str, client_id: &str) -> NewOrder {
    NewOrder {
        protect: None,
        symbol: "BTC-USD".into(),
        side: Side::Sell,
        order_type: OrderType::PostOnly,
        price: d(price),
        quantity: d("0.25"),
        client_id: Some(client_id.into()),
        reduce_only: true,
    }
}

/// What the echo strategy saw, shared with the test (the strategy lives in a thread-local).
static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Echoes what it sees: every call is noted, every event answers one intent of each kind.
#[derive(Debug)]
struct Echo {
    ticks: u32,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
struct EchoReport {
    ticks: u32,
}

impl StrategyReport for EchoReport {
    fn headline(&self) -> String {
        format!("{} ticks", self.ticks)
    }

    fn fields(&self) -> Vec<ReportField> {
        vec![
            ReportField::number("ticks", Decimal::from(self.ticks)),
            ReportField::text("mode", "echo").unit("word"),
            ReportField::flag("on", true),
        ]
    }

    fn paper_inputs(&self) -> Option<PaperInputs> {
        Some(PaperInputs { order_size: d("0.25"), quoting_secs: 7, maker_fee_bps: d("-0.5") })
    }

    fn briefing_lines(&self) -> Vec<String> {
        vec!["It echoes. ".into()]
    }

    fn reference_price(&self) -> Option<Decimal> {
        Some(d("75.5"))
    }
}

impl FromSettings for Echo {
    fn from_settings(settings: &str) -> Result<Self, StrategyError> {
        match settings {
            "refuse" => Err(StrategyError("echo: refused".into())),
            _ => Ok(Echo { ticks: 0 }),
        }
    }
}

fn note(what: String) {
    SEEN.lock().unwrap().push(what);
}

impl StrategyAdapter for Echo {
    type Report = EchoReport;

    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        note(format!("start {view:?}"));
        Ok(vec![Intent::CancelAll])
    }

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent> {
        note(format!("{event:?}"));
        note(format!("{view:?}"));
        self.ticks = self.ticks.saturating_add(1);
        let cancel =
            CancelOrder { symbol: "BTC-USD".into(), order_id: "9".into(), client_id: None };
        vec![
            Intent::Place(order("76.125", "a")),
            Intent::Cancel(cancel.clone()),
            Intent::Amend(AmendOrder { cancel, new_order: order("-1.5", "b") }),
            Intent::Shutdown("done".into()),
            Intent::Leg { leg: "hedge".into(), intent: Box::new(Intent::Place(order("2", "h"))) },
            Intent::Flatten { symbols: vec!["ETH".into()] },
            Intent::Request {
                id: 7,
                leg: "trade".into(),
                call: VenueCall::Read(ReadQuery::OpenOrders { symbol: "BTC-USD".into() }),
            },
            // A shutdown on a leg crosses as a plain shutdown, as the native host lifts it.
            Intent::Leg { leg: "hedge".into(), intent: Box::new(Intent::Shutdown("x".into())) },
        ]
    }

    fn stop(&mut self, _view: &View<'_>) -> Vec<Intent> {
        vec![Intent::CancelAll]
    }

    fn report(&self) -> EchoReport {
        EchoReport { ticks: self.ticks }
    }
}

fn book() -> OrderBook {
    OrderBook {
        bids: [(d("74.9"), d("3")), (d("75"), d("1.5"))].into(),
        asks: [(d("76"), d("2")), (d("76.1"), d("4"))].into(),
        last_update_id: 42,
    }
}

fn resting() -> Vec<Order> {
    vec![Order {
        id: "9".into(),
        client_id: Some("a".into()),
        symbol: "BTC-USD".into(),
        side: Side::Buy,
        order_type: OrderType::Limit,
        price: d("75"),
        quantity: d("1"),
        filled_quantity: d("0.5"),
        status: OrderStatus::PartiallyFilled,
    }]
}

fn hedge() -> LegView {
    let mut leg = LegView::new("hedge", "hyperliquid", vec!["ETH".into()]);
    leg.books.insert("ETH".into(), std::sync::Arc::new(book()));
    leg.book_at_ms.insert("ETH".into(), 1_794_398_399_000);
    leg.marks.insert("ETH".into(), d("2500.5"));
    leg.funding_rates.insert("ETH".into(), d("0.0001"));
    leg.reconnected = true;
    leg.positions = Some(vec![position()]);
    leg.market
        .insert("ETH".into(), MarketFacts { tick_size: Some(d("0.01")), ..MarketFacts::default() });
    leg
}

fn position() -> Position {
    Position {
        symbol: "ETH".into(),
        side: Some(Side::Sell),
        size: d("1.5"),
        entry_price: d("2400"),
        unrealized_pnl: d("-150.75"),
    }
}

fn view<'a>(book: &'a OrderBook, orders: &'a [Order], legs: &'a [LegView]) -> View<'a> {
    View {
        now_ms: 1_794_398_400_000,
        symbol: "BTC-USD",
        book: Some(book),
        positions: None,
        reconnected: true,
        venue_halt: Some("stale oracle"),
        legs,
        path_stop: Some("daily loss"),
        read_started_ms: Some(1_794_398_399_900),
        limits: None,
        position: d("-0.75"),
        avg_entry: Some(d("75.25")),
        realized_pnl: d("-12.5"),
        open_orders: Some(orders),
        market: MarketFacts {
            tick_size: Some(d("0.1")),
            step_size: None,
            maker_fee_bps: Some(d("-0.5")),
            taker_fee_bps: Some(d("4.5")),
            fee_tier: Some(3),
            venue_name: Some("Bullet".into()),
        },
    }
}

/// One test: the strategy is a thread-local, so the calls run in order on one thread.
#[test]
fn every_call_crosses_the_wire_and_back_unchanged() {
    assert_eq!(
        start(&wire::host::view(&view(&book(), &[], &[]))).err().as_deref(),
        Some(NOT_STARTED)
    );
    assert_eq!(init::<Echo>("refuse"), Err("echo: refused".into()));
    init::<Echo>("{}").unwrap();

    let (book, orders, legs) = (book(), resting(), [hedge()]);
    let sent = view(&book, &orders, &legs);
    let started = start(&wire::host::view(&sent)).unwrap();
    assert!(matches!(started[..], [w::Intent::CancelAll]));
    assert_eq!(SEEN.lock().unwrap().pop(), Some(format!("start {sent:?}")), "the view as sent");

    let fill = Event::Fill(Trade {
        exchange: "bullet".into(),
        symbol: "BTC-USD".into(),
        order_id: "9".into(),
        trade_id: Some("t1".into()),
        client_id: Some("a".into()),
        side: Side::Buy,
        price: d("75"),
        quantity: d("0.5"),
        timestamp: Some(1_794_398_400_123),
    });
    let refused = Event::Outcome {
        intent: Intent::Place(order("76.125", "a")),
        result: OrderResult {
            order_id: None,
            client_id: Some("a".into()),
            success: false,
            error: Some("risk gate".into()),
        },
    };
    let order_event = Event::Order(resting().remove(0));
    for event in [Event::Tick, Event::Book, Event::Mark(d("75.25")), fill, refused, order_event]
        .into_iter()
        .chain(new_events())
    {
        let intents = on_event(wire::host::event(&event).unwrap(), &wire::host::view(&sent));
        let back: Vec<Intent> =
            intents.into_iter().map(|i| wire::intent_from_wire(i).unwrap()).collect();
        assert_eq!(back[0], Intent::Place(order("76.125", "a")));
        assert_eq!(
            back[1],
            Intent::Cancel(CancelOrder {
                symbol: "BTC-USD".into(),
                order_id: "9".into(),
                client_id: None
            })
        );
        assert!(matches!(&back[2], Intent::Amend(a) if a.new_order.price == d("-1.5")));
        assert_eq!(back[3], Intent::Shutdown("done".into()));
        assert_eq!(
            back[4],
            Intent::Leg { leg: "hedge".into(), intent: Box::new(Intent::Place(order("2", "h"))) }
        );
        assert_eq!(back[5], Intent::Flatten { symbols: vec!["ETH".into()] });
        assert!(matches!(&back[6], Intent::Request { id: 7, .. }));
        assert_eq!(back.len(), 8, "a shutdown on a leg crosses as a plain one, never dropped");
        assert_eq!(back[7], Intent::Shutdown("x".into()));
        let mut seen = SEEN.lock().unwrap();
        assert_eq!(seen.pop(), Some(format!("{sent:?}")));
        assert_eq!(seen.pop(), Some(format!("{event:?}")), "the event as sent");
    }

    let record = report();
    let fields: Vec<(String, FieldValue, Option<String>)> = record
        .fields
        .into_iter()
        .map(|f| {
            let value = match f.value {
                w::FieldValue::Number(n) => FieldValue::Number(wire::undec(n).unwrap()),
                w::FieldValue::Text(t) => FieldValue::Text(t),
                w::FieldValue::Flag(b) => FieldValue::Flag(b),
            };
            (f.name, value, f.unit)
        })
        .collect();
    let want = [
        ("ticks".into(), FieldValue::Number(d("22")), None),
        ("mode".into(), FieldValue::Text("echo".into()), Some("word".into())),
        ("on".into(), FieldValue::Flag(true), None),
    ];
    assert_eq!(fields, want, "each field as the strategy gave it");
    assert_eq!(record.headline, "22 ticks");
    assert_eq!(record.briefing_lines, ["It echoes. "]);
    let inputs = record.paper_inputs.unwrap();
    assert_eq!(wire::undec(inputs.maker_fee_bps).unwrap(), d("-0.5"));
    assert_eq!(inputs.quoting_secs, 7);
    assert_eq!(record.reference_price.map(|p| wire::undec(p).unwrap()), Some(d("75.5")));
    assert!(matches!(stop(&wire::host::view(&sent))[..], [w::Intent::CancelAll]));
}

impl Describe for Echo {
    const NAME: &'static str = "echo";
    const VERSION: &'static str = "1.2.3";

    fn describe() -> crate::strategy::describe::Descriptor {
        use crate::strategy::describe::{Range, SettingKind, SettingSpec, Tier};
        let mode = SettingSpec {
            kind: SettingKind::Choice(vec!["a".into(), "b".into()]),
            ..SettingSpec::int("mode", 1, Range::new(d("0"), d("1")), "Which.")
        };
        crate::strategy::describe::Descriptor {
            summary: "Echoes.".into(),
            settings: vec![
                SettingSpec::decimal("size", d("0.25"), Range::default(), "One order.")
                    .tier(Tier::Limit)
                    .loosening(crate::strategy::describe::Loosening::Up)
                    .required(),
                mode,
            ],
            settings_schema: schemars::schema_for!(EchoReport),
            legs: vec![crate::strategy::describe::LegNeed::new("hedge", "hyperliquid")],
        }
    }
}

#[test]
fn the_descriptor_crosses_with_its_ranges_and_schema() {
    let wire = describe::<Echo>();
    assert_eq!((wire.name.as_str(), wire.version.as_str()), ("echo", "1.2.3"));
    let size = &wire.settings[0];
    assert_eq!(size.path, "size");
    assert!(matches!(size.kind, w::SettingKind::Decimal) && size.required);
    assert!(matches!(size.tier, w::Tier::Limit));
    assert!(matches!(size.loosening, w::Loosening::Up));
    assert_eq!(wire::undec(size.default).unwrap(), d("0.25"));
    assert!(size.hard.min.is_none() && size.hard.max.is_none());
    let mode = &wire.settings[1];
    assert!(matches!(&mode.kind, w::SettingKind::Choice(c) if c == &["a", "b"]));
    assert_eq!(mode.guarded.max.map(|m| wire::undec(m).unwrap()), Some(d("1")));
    let schema: serde_json::Value = serde_json::from_str(&wire.settings_schema).unwrap();
    assert!(schema["properties"]["ticks"].is_object());
    assert_eq!(wire.legs.len(), 1);
    assert_eq!((wire.legs[0].id.as_str(), wire.legs[0].venue.as_str()), ("hedge", "hyperliquid"));
}

/// The section the export macro writes (on wasm32): `name`, a newline, `version`; and `describe`
/// reports the same constants.
#[test]
fn the_meta_section_holds_the_name_and_version_describe_reports() {
    const BYTES: [u8; 10] = meta("echo", "1.2.3");
    assert_eq!(&BYTES, b"echo\n1.2.3");
    let d = describe::<Echo>();
    assert_eq!((d.name.as_str(), d.version.as_str()), (Echo::NAME, Echo::VERSION));
}

/// Every event the 0.3.0 world added, each case of each answer included.
fn new_events() -> Vec<Event> {
    let trade = Trade {
        exchange: "hyperliquid".into(),
        symbol: "ETH".into(),
        order_id: "h1".into(),
        trade_id: None,
        client_id: Some("h".into()),
        side: Side::Buy,
        price: d("2500"),
        quantity: d("0.1"),
        timestamp: None,
    };
    let on =
        |event| Event::Leg { leg: "hedge".into(), symbol: "ETH".into(), event: Box::new(event) };
    let answer = |id, reply| Event::Answer { id, reply };
    let equity =
        AccountEquity { equity: d("100"), wallet_balance: d("90"), unrealized_pnl: d("10") };
    let placed =
        OrderResult { order_id: Some("1".into()), client_id: None, success: true, error: None };
    let cancelled =
        CancelResult { order_id: "1".into(), success: false, error: Some("gone".into()) };
    vec![
        on(Event::Book),
        on(Event::Mark(d("2500.5"))),
        on(Event::Fill(trade)),
        on(Event::Order(resting().remove(0))),
        answer(1, Ok(Answer::OpenOrders(resting()))),
        answer(2, Ok(Answer::Positions(vec![position()]))),
        answer(3, Ok(Answer::Book(book()))),
        answer(4, Ok(Answer::AccountEquity(Some(equity)))),
        answer(5, Ok(Answer::MarginRatio(Some(d("0.2"))))),
        answer(
            6,
            Ok(Answer::FundingHistory(vec![FundingSample { time_ms: -5, rate_8h: d("0.0001") }])),
        ),
        answer(7, Ok(Answer::OpenInterest(None))),
        answer(8, Ok(Answer::Placed(vec![placed]))),
        answer(9, Ok(Answer::Cancelled(vec![cancelled]))),
        answer(10, Ok(Answer::Done)),
        answer(11, Err(CallError::new("timed out", false, true))),
        Event::Unknown { intent: Intent::Place(order("76.125", "a")), error: "timeout".into() },
    ]
}

#[test]
fn every_venue_call_crosses_both_ways() {
    let symbol = || "ETH".to_string();
    let calls = [
        VenueCall::Read(ReadQuery::Positions),
        VenueCall::Read(ReadQuery::OpenOrders { symbol: symbol() }),
        VenueCall::Read(ReadQuery::Book { symbol: symbol(), depth: 20 }),
        VenueCall::Read(ReadQuery::AccountEquity),
        VenueCall::Read(ReadQuery::MarginRatio),
        VenueCall::Read(ReadQuery::FundingHistory { symbol: symbol(), start_ms: -1 }),
        VenueCall::Read(ReadQuery::OpenInterest { symbol: symbol() }),
        VenueCall::Read(ReadQuery::RefreshMetadata),
        VenueCall::Place(vec![order("1", "p")]),
        VenueCall::Cancel(vec![CancelOrder {
            symbol: symbol(),
            order_id: "1".into(),
            client_id: None,
        }]),
        VenueCall::Amend(vec![AmendOrder {
            cancel: CancelOrder { symbol: symbol(), order_id: "1".into(), client_id: None },
            new_order: order("2", "q"),
        }]),
        VenueCall::CancelAll { symbol: symbol() },
        VenueCall::Wait { until_ms: 9 },
    ];
    for call in calls {
        let intent = Intent::Request { id: 3, leg: "hedge".into(), call };
        let wired = wire::intent_to_wire(intent.clone()).unwrap().unwrap();
        assert_eq!(wire::intent_from_wire(wired).unwrap(), intent);
    }
}
