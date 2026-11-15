//! The execution layer (new code of this plugin; `decide` is the parity tests' job): what it reads,
//! what it sends and what it refuses to.

use bb_strategy_sdk::testing::{Market, book, d};
use bb_strategy_sdk::{
    AccountEquity, Answer, CallError, Event, FromSettings, FundingSample, Intent, LegView,
    MarketFacts, NewOrder, OrderResult, Position, ReadQuery, Side, StrategyAdapter, TRADE_LEG,
    VenueCall,
};

use super::*;

const SYM: &str = "BTC-USD";
const HSYM: &str = "BTC";
const T0: u64 = 1_800_000_000_000;

/// No intents.
fn none() -> Vec<Intent> {
    Vec::new()
}

fn strategy() -> FundingArb {
    FundingArb::from_settings("{}").unwrap()
}

fn facts() -> MarketFacts {
    MarketFacts {
        step_size: Some(d("0.001")),
        taker_fee_bps: Some(d("0")),
        ..MarketFacts::default()
    }
}

/// Both books 10 bp wide at 100, marks fresh, steps 0.001.
fn market(now: u64) -> Market {
    let mut m = Market::new(SYM, "99.95", "100.05", now);
    m.market = facts();
    let mut leg = LegView::new(HEDGE_LEG, "hyperliquid", vec![HSYM.into()]);
    leg.books.insert(HSYM.into(), book("99.95", "100.05", "100"));
    leg.book_at_ms.insert(HSYM.into(), now);
    leg.market.insert(HSYM.into(), facts());
    m.legs = vec![leg];
    m
}

fn funding(rate: &str) -> Vec<FundingSample> {
    (0..24).map(|i| FundingSample { time_ms: i, rate_8h: d(rate) }).collect()
}

fn position(symbol: &str, side: Side, qty: &str) -> Position {
    Position {
        symbol: symbol.into(),
        side: Some(side),
        size: d(qty),
        entry_price: d("100"),
        unrealized_pnl: d("0"),
    }
}

/// What the two venues hold and say.
struct Venues {
    bullet: Vec<Position>,
    hl: Vec<Position>,
    funding_bullet: &'static str,
    funding_hl: &'static str,
    fail: Option<&'static str>,
}

impl Venues {
    fn flat() -> Self {
        Self { bullet: vec![], hl: vec![], funding_bullet: "0.0005", funding_hl: "0", fail: None }
    }

    fn answer(&self, leg: &str, call: &VenueCall) -> Result<Answer, CallError> {
        let VenueCall::Read(q) = call else { panic!("a round only reads: {call:?}") };
        if self.fail.is_some_and(|f| format!("{q:?}").contains(f)) {
            return Err(CallError { message: "down".into(), never_sent: true, retryable: true });
        }
        let on_bullet = leg == TRADE_LEG;
        Ok(match q {
            ReadQuery::Positions => {
                Answer::Positions(if on_bullet { self.bullet.clone() } else { self.hl.clone() })
            }
            ReadQuery::AccountEquity => Answer::AccountEquity(Some(AccountEquity {
                equity: d("10000"),
                wallet_balance: d("10000"),
                unrealized_pnl: d("0"),
            })),
            ReadQuery::FundingHistory { .. } => Answer::FundingHistory(funding(if on_bullet {
                self.funding_bullet
            } else {
                self.funding_hl
            })),
            ReadQuery::OpenInterest { .. } => Answer::OpenInterest(Some(d("1000000"))),
            other => panic!("unexpected read {other:?}"),
        })
    }
}

/// One tick, every read answered, and what the last answer made the strategy send.
fn round(s: &mut FundingArb, m: &Market, v: &Venues) -> Vec<Intent> {
    let asks = s.on_event(&Event::Tick, &m.view());
    assert_eq!(asks.len(), 7, "{asks:?}");
    let mut out = Vec::new();
    for ask in asks {
        let Intent::Request { id, leg, call } = ask else { panic!("a request: {ask:?}") };
        let reply = v.answer(&leg, &call);
        out = s.on_event(&Event::Answer { id, reply }, &m.view());
    }
    out
}

fn placed(intents: &[Intent]) -> Vec<(&str, &NewOrder)> {
    intents
        .iter()
        .filter_map(|i| match i {
            Intent::Request { leg, call: VenueCall::Place(os), .. } => {
                Some(os.iter().map(move |o| (leg.as_str(), o)))
            }
            _ => None,
        })
        .flatten()
        .collect()
}

/// Answer the order requests in `intents` as the venue would, then wait out the settle pause.
fn settle(s: &mut FundingArb, m: &mut Market, intents: &[Intent], ok: bool) {
    for i in intents {
        if let Intent::Request { id, .. } = i {
            let result = OrderResult {
                order_id: ok.then(|| "o".to_owned()),
                client_id: None,
                success: ok,
                error: (!ok).then(|| "refused".to_owned()),
            };
            let reply = Ok(Answer::Placed(vec![result]));
            s.on_event(&Event::Answer { id: *id, reply }, &m.view());
        }
    }
    m.now_ms += 6_000;
    fresh(m);
}

fn fresh(m: &mut Market) {
    m.legs[0].book_at_ms.insert(HSYM.into(), m.now_ms);
}

/// A started strategy with a fresh Bullet book and mark.
fn ready() -> (FundingArb, Market) {
    let mut s = strategy();
    let m = market(T0);
    s.start(&m.view()).unwrap();
    s.on_event(&Event::Book, &m.view());
    s.on_event(&Event::Mark(d("100")), &m.view());
    (s, m)
}

#[test]
fn start_needs_a_hedge_leg_with_one_symbol() {
    let mut m = market(T0);
    m.legs[0].symbols.clear();
    assert!(strategy().start(&m.view()).unwrap_err().0.contains("hedge leg"));
    m.legs.clear();
    assert!(strategy().start(&m.view()).is_err());
    assert_eq!(strategy().start(&market(T0).view()).unwrap(), none());
}

#[test]
fn a_round_reads_both_venues_and_trades_nothing_with_no_edge() {
    let (mut s, m) = ready();
    let mut v = Venues::flat();
    v.funding_bullet = "0.00001";
    assert_eq!(round(&mut s, &m, &v), none());
    let r = s.report();
    assert_eq!(r.target_usd, Some(d("0")));
    assert_eq!(r.bullet_base, Some(d("0")));
}

#[test]
fn an_edge_on_a_flat_pair_sells_bullet_first_with_a_capped_ioc() {
    let (mut s, m) = ready();
    let out = round(&mut s, &m, &Venues::flat());
    let orders = placed(&out);
    assert_eq!(orders.len(), 1, "{out:?}");
    let (leg, o) = orders[0];
    assert_eq!(
        (leg, o.symbol.as_str(), o.side, o.reduce_only),
        (TRADE_LEG, SYM, Side::Sell, false)
    );
    assert_eq!(o.quantity, d("1"), "100 USD at 100");
    assert_eq!(o.price, d("99.95") * d("0.999"), "the touch less slippage_bps");
    assert_eq!(o.order_type, OrderType::Market);
}

#[test]
fn the_hedge_is_sized_to_what_bullet_filled_and_goes_the_other_way() {
    let (mut s, mut m) = ready();
    let first = round(&mut s, &m, &Venues::flat());
    settle(&mut s, &mut m, &first, true);
    let mut v = Venues::flat();
    v.bullet = vec![position(SYM, Side::Sell, "0.6")];
    let out = round(&mut s, &m, &v);
    let orders = placed(&out);
    assert_eq!(orders.len(), 1, "{out:?}");
    let (leg, o) = orders[0];
    assert_eq!(
        (leg, o.symbol.as_str(), o.side, o.quantity),
        (HEDGE_LEG, HSYM, Side::Buy, d("0.6"))
    );
    assert!(!o.reduce_only);
}

#[test]
fn a_refused_hedge_unwinds_the_bullet_leg_reduce_only() {
    let (mut s, mut m) = ready();
    let first = round(&mut s, &m, &Venues::flat());
    settle(&mut s, &mut m, &first, true);
    let mut v = Venues::flat();
    v.bullet = vec![position(SYM, Side::Sell, "1")];
    let hedge_orders = round(&mut s, &m, &v);
    settle(&mut s, &mut m, &hedge_orders, false);
    // The hedge never filled: the next round still sees Bullet alone.
    let out = round(&mut s, &m, &v);
    let orders = placed(&out);
    assert_eq!(orders.len(), 1, "{out:?}");
    let (leg, o) = orders[0];
    assert_eq!((leg, o.side, o.quantity, o.reduce_only), (TRADE_LEG, Side::Buy, d("1"), true));
    assert_eq!(s.report().unwind_attempts, 1);
}

#[test]
fn a_one_sided_book_nobody_sent_is_closed_not_hedged() {
    let (mut s, m) = ready();
    let mut v = Venues::flat();
    v.hl = vec![position(HSYM, Side::Buy, "2")];
    let out = round(&mut s, &m, &v);
    let orders = placed(&out);
    assert_eq!(orders.len(), 1, "{out:?}");
    assert_eq!(
        (orders[0].0, orders[0].1.side, orders[0].1.reduce_only),
        (HEDGE_LEG, Side::Sell, true)
    );
}

#[test]
fn a_matched_pair_with_the_edge_still_there_is_left_alone() {
    let (mut s, m) = ready();
    let mut v = Venues::flat();
    v.bullet = vec![position(SYM, Side::Sell, "1")];
    v.hl = vec![position(HSYM, Side::Buy, "0.95")];
    assert_eq!(round(&mut s, &m, &v), none(), "within the tolerance, never grown or trimmed");
}

#[test]
fn a_signal_that_flips_closes_both_legs() {
    let (mut s, m) = ready();
    let mut v = Venues::flat();
    v.funding_bullet = "-0.0005";
    v.bullet = vec![position(SYM, Side::Sell, "1")];
    v.hl = vec![position(HSYM, Side::Buy, "1")];
    let out = round(&mut s, &m, &v);
    let orders = placed(&out);
    assert_eq!(orders.len(), 2, "{out:?}");
    assert!(orders.iter().all(|(_, o)| o.reduce_only));
    assert_eq!((orders[0].0, orders[0].1.side), (TRADE_LEG, Side::Buy));
    assert_eq!((orders[1].0, orders[1].1.side), (HEDGE_LEG, Side::Sell));
}

#[test]
fn legs_that_do_not_match_are_both_closed() {
    let (mut s, m) = ready();
    let mut v = Venues::flat();
    v.bullet = vec![position(SYM, Side::Sell, "1")];
    v.hl = vec![position(HSYM, Side::Buy, "0.5")];
    assert_eq!(placed(&round(&mut s, &m, &v)).len(), 2);
    // The same side on both venues is no hedge at all.
    let (mut s, m) = ready();
    v.hl = vec![position(HSYM, Side::Sell, "1")];
    assert_eq!(placed(&round(&mut s, &m, &v)).len(), 2);
}

#[test]
fn a_failed_read_trades_nothing_and_tries_again_soon() {
    let (mut s, mut m) = ready();
    let mut v = Venues::flat();
    v.fail = Some("OpenInterest");
    assert_eq!(round(&mut s, &m, &v), none());
    assert!(s.report().note.unwrap().contains("a read failed"));
    // Not a flat book: an unreadable position never opens anything.
    m.now_ms += 5_000;
    fresh(&mut m);
    s.on_event(&Event::Book, &m.view());
    s.on_event(&Event::Mark(d("100")), &m.view());
    v.fail = Some("Positions");
    assert_eq!(round(&mut s, &m, &v), none());
}

#[test]
fn entries_wait_for_fresh_books_and_a_clear_gate() {
    // The hedge book is older than the freshness window.
    let (mut s, mut m) = ready();
    m.legs[0].book_at_ms.insert(HSYM.into(), T0 - 31_000);
    assert_eq!(round(&mut s, &m, &Venues::flat()), none());
    // The bot's gate says stop.
    let (mut s, mut m) = ready();
    m.path_stop = Some("daily loss".into());
    assert_eq!(round(&mut s, &m, &Venues::flat()), none());
    assert!(s.report().note.unwrap().contains("daily loss"));
    // No mark seen.
    let mut s = strategy();
    let m = market(T0);
    s.start(&m.view()).unwrap();
    s.on_event(&Event::Book, &m.view());
    assert_eq!(round(&mut s, &m, &Venues::flat()), none());
}

#[test]
fn the_next_round_waits_for_its_interval() {
    let (mut s, mut m) = ready();
    let mut v = Venues::flat();
    v.funding_bullet = "0.00001";
    assert_eq!(round(&mut s, &m, &v), none());
    m.now_ms += 10_000;
    assert_eq!(s.on_event(&Event::Tick, &m.view()), none(), "300 s between quiet rounds");
    m.now_ms += 300_000;
    assert_eq!(s.on_event(&Event::Tick, &m.view()).len(), 7);
}

#[test]
fn stop_flattens_both_legs() {
    let (mut s, m) = ready();
    let out = s.stop(&m.view());
    let legs: Vec<&str> = out
        .iter()
        .filter_map(|i| if let Intent::Leg { leg, .. } = i { Some(leg.as_str()) } else { None })
        .collect();
    assert_eq!(legs, [TRADE_LEG, HEDGE_LEG]);
}

#[test]
fn the_settings_are_the_built_ins_and_refuse_what_it_refuses() {
    assert!(FundingArb::from_settings(r#"{"max_leverage": "6"}"#).is_err());
    assert!(FundingArb::from_settings(r#"{"max_oi_pct": "0.6"}"#).is_err());
    assert!(FundingArb::from_settings(r#"{"expected_hold_hours": 721}"#).is_err());
    assert!(FundingArb::from_settings(r#"{"max_positions": 0}"#).is_err());
    assert!(FundingArb::from_settings(r#"{"dry_run": true}"#).is_err(), "the bot's, not a setting");
    let s: FundingArbSettings = serde_json::from_str("{}").expect("defaults");
    assert_eq!((s.notional_per_symbol, s.max_positions, s.expected_hold_hours), (d("100"), 8, 72));
}
