//! The ladder on the SDK's simulated host: no venue, no component.

use bb_strategy_sdk::testing::{Market, SimHost, d, fill_of, outcome, placed};
use bb_strategy_sdk::{
    Event, FieldValue, Intent, NewOrder, OrderType, Side, StrategyAdapter, StrategyReport,
};

use super::*;

const NOON: u64 = 1_794_398_400_000;

fn settings(levels: u32) -> LadderSettings {
    LadderSettings {
        levels,
        spacing_bps: d("10"),
        order_size: d("0.01"),
        order_type: OrderType::PostOnly,
    }
}

fn host(levels: u32, market: Market) -> SimHost<Ladder> {
    SimHost::new(Ladder::new(settings(levels)).unwrap(), market)
}

fn prices(intents: &[Intent]) -> Vec<(Side, String)> {
    placed(intents).iter().map(|o| (o.side, o.price.to_string())).collect()
}

#[test]
fn start_lays_the_ladder_around_the_mid_rounded_away_from_it() {
    let mut sim = host(2, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let want = [
        (Side::Buy, "75.424"),
        (Side::Buy, "75.349"),
        (Side::Sell, "75.576"),
        (Side::Sell, "75.651"),
    ];
    let want: Vec<(Side, String)> = want.iter().map(|(s, p)| (*s, (*p).to_string())).collect();
    assert_eq!(prices(&sent), want);
    assert!(placed(&sent).iter().all(|o| o.quantity == d("0.01") && o.symbol == "BTC-USD"));
    let report = sim.strategy.report();
    assert!(report.levels.iter().all(|r| r.state == LevelState::Resting), "{report:?}");
    assert_eq!(report.headline(), "ladder around 75.5: 4 resting, 0 pending, 0 refused, 0 fills");
}

#[test]
fn the_venue_tick_size_rounds_the_levels() {
    let mut market = Market::new("BTC-USD", "75", "76", NOON);
    market.market.tick_size = Some(d("0.5"));
    let mut sim = host(1, market);
    let sent = sim.start().unwrap();
    assert_eq!(prices(&sent), [(Side::Buy, "75".to_string()), (Side::Sell, "76".to_string())]);
}

#[test]
fn without_a_book_it_waits_and_lays_the_ladder_on_the_first() {
    let mut market = Market::new("BTC-USD", "75", "76", NOON);
    market.book = None;
    let mut sim = host(1, market);
    assert_eq!(sim.start().unwrap().len(), 0);
    assert_eq!(sim.strategy.report().headline(), "ladder: waiting for a book");
    sim.market.book = Some(bb_strategy_sdk::testing::book("99", "101", "1"));
    let sent = sim.send(&Event::Book);
    assert_eq!(prices(&sent), [(Side::Buy, "99.9".to_string()), (Side::Sell, "100.1".to_string())]);
    assert_eq!(sim.send(&Event::Book).len(), 0, "laid once");
}

#[test]
fn a_filled_level_is_placed_again_at_its_price_with_a_new_client_id() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let buy = placed(&sent)[0].clone();
    let again = sim.send(&fill_of(&buy));
    let again = placed(&again);
    assert_eq!(again.len(), 1);
    assert_eq!((again[0].side, again[0].price), (buy.side, buy.price));
    assert_ne!(again[0].client_id, buy.client_id);
    assert_eq!(sim.market.position, d("0.01"));
    assert_eq!(sim.strategy.report().fills, 1);
}

#[test]
fn a_refused_level_is_not_placed_again() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    sim.refuse = |o| (o.side == Side::Buy).then_some("risk gate: max position");
    let sent = sim.start().unwrap();
    assert_eq!(placed(&sent).len(), 2);
    for _ in 0..3 {
        assert_eq!(sim.send(&Event::Tick).len(), 0);
    }
    let states: Vec<LevelState> = sim.strategy.report().levels.iter().map(|r| r.state).collect();
    assert_eq!(states, [LevelState::Refused, LevelState::Resting]);
}

#[test]
fn a_cancelled_level_counts_as_refused() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let sell = placed(&sent)[1].clone();
    let cancelled = bb_strategy_sdk::Order {
        id: "x".into(),
        client_id: sell.client_id.clone(),
        symbol: sell.symbol.clone(),
        side: sell.side,
        order_type: sell.order_type,
        price: sell.price,
        quantity: sell.quantity,
        filled_quantity: d("0"),
        status: OrderStatus::Cancelled,
    };
    assert_eq!(sim.send(&Event::Order(cancelled)).len(), 0);
    assert_eq!(sim.strategy.report().levels[1].state, LevelState::Refused);
}

#[test]
fn an_outcome_for_an_unknown_order_changes_nothing() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    sim.start().unwrap();
    let before = sim.strategy.report();
    let mut stranger = placed(&sim.sent)[0].clone();
    stranger.client_id = Some("not-ours".into());
    assert_eq!(sim.send(&outcome(&Intent::Place(stranger), Err("refused"))).len(), 0);
    assert_eq!(sim.strategy.report(), before);
}

#[test]
fn stop_cancels_everything() {
    let mut sim = host(2, Market::new("BTC-USD", "75", "76", NOON));
    sim.start().unwrap();
    assert_eq!(sim.stop(), [Intent::CancelAll]);
}

#[test]
fn the_report_shows_its_fields_and_feeds_the_paper_report() {
    let mut sim = host(2, Market::new("BTC-USD", "75", "76", NOON));
    sim.start().unwrap();
    sim.market.now_ms = NOON.saturating_add(90_000);
    sim.send(&Event::Tick);
    let report = sim.strategy.report();
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["mid"], "75.5");
    assert_eq!(json["levels"][2]["side"], "sell");
    assert_eq!(json["levels"][2]["state"], "resting");
    let fields = report.fields();
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        ["mid", "fills", "order_size", "quoting_secs", "buy 1", "buy 2", "sell 1", "sell 2"]
    );
    assert_eq!(fields[0], ReportField::number("mid", d("75.5")));
    assert_eq!(fields[3], ReportField::number("quoting_secs", d("90")).unit("s"));
    assert!(
        matches!(&fields[6].value, FieldValue::Text(t) if t.starts_with("resting at ")),
        "{:?}",
        fields[6]
    );
    let inputs = report.paper_inputs().unwrap();
    assert_eq!((inputs.order_size, inputs.quoting_secs), (d("0.01"), 90));
    assert_eq!(report.reference_price(), Some(d("75.5")));
    assert_eq!(
        report.briefing_lines(),
        ["The ladder has 2 resting buys and 2 resting sells around the mid. "]
    );
}

#[test]
fn settings_it_cannot_run_with_are_refused() {
    for bad in [
        r#"{"levels": 0, "spacing_bps": "10", "order_size": "1"}"#,
        r#"{"levels": 33, "spacing_bps": "10", "order_size": "1"}"#,
        r#"{"levels": 1, "spacing_bps": "0", "order_size": "1"}"#,
        r#"{"levels": 1, "spacing_bps": "10", "order_size": "0"}"#,
        r#"{"levels": 1, "spacing_bps": "10", "order_size": "1", "extra": 1}"#,
        "not json",
    ] {
        assert!(Ladder::from_settings(bad).is_err(), "{bad}");
    }
    let ok = Ladder::from_settings(r#"{"levels": 3, "spacing_bps": "25", "order_size": "0.5"}"#);
    assert_eq!(ok.unwrap().settings.order_type, OrderType::PostOnly);
}

#[test]
fn a_partial_fill_leaves_the_rest_of_the_order_resting_and_places_nothing() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    let sent = sim.start().unwrap();
    let mut half = placed(&sent)[0].clone();
    half.quantity = d("0.004");
    assert_eq!(sim.send(&fill_of(&half)).len(), 0, "0.004 of 0.01 filled: the order still rests");
    assert_eq!(sim.strategy.report().fills, 0);
    half.quantity = d("0.006");
    assert_eq!(placed(&sim.send(&fill_of(&half))).len(), 1, "now it is whole: placed again");
    assert_eq!(sim.strategy.report().fills, 1);
}

#[test]
fn settings_that_could_never_lay_the_whole_ladder_are_refused() {
    let mut far = settings(2);
    far.spacing_bps = d("6000");
    assert!(Ladder::new(far).unwrap_err().0.contains("farthest buy"));
    let mut market = settings(1);
    market.order_type = OrderType::Market;
    assert!(Ladder::new(market).unwrap_err().0.contains("not market"));
}

#[test]
fn quoting_time_and_the_briefing_count_only_resting_levels() {
    let mut sim = host(1, Market::new("BTC-USD", "75", "76", NOON));
    sim.refuse = |o| (o.side == Side::Buy).then_some("risk gate");
    sim.start().unwrap();
    sim.market.now_ms = NOON + 10_000;
    sim.send(&Event::Tick);
    let report = sim.strategy.report();
    assert_eq!(report.quoting_secs, 10);
    assert_eq!(
        report.briefing_lines(),
        ["The ladder has 0 resting buys and 1 resting sells around the mid. "]
    );
}

#[path = "retry_tests.rs"]
mod retry;

#[path = "unlisted_tests.rs"]
mod unlisted;
