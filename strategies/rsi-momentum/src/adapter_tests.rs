//! The strategy alone, over hand-built views (the host's tests, with a mock venue, are the bot's).

#![expect(clippy::arithmetic_side_effects, reason = "test arithmetic")]

use std::collections::BTreeMap;

use bb_strategy_sdk::{OrderBook, Position, Side, ValidateConfig};
use rust_decimal::Decimal;

use crate::adapter::RsiMomentum;
use crate::config::RsiMomentumConfig;

const SYM: &str = "BTC-USD";
const MIN: u64 = 60_000;
/// A UTC day boundary.
const T0: u64 = 20_000 * 1_440 * MIN;

/// Period-2 closes: the RSI reads 100, 100, 20 (a cross below 40), then 73.3 (a cross above 60)
/// at the close of candle 5.
const LONG: [&str; 6] = ["100", "101", "102", "103", "99", "104"];

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

/// A config on 1m candles, RSI period 2, 100 USD entries, no cooldown and a wide stop, unless
/// `extra` sets them.
fn config(extra: &str) -> RsiMomentumConfig {
    let mut table: toml::Table = toml::from_str(extra).unwrap();
    for (key, value) in [
        ("symbol", toml::Value::from(SYM)),
        ("interval", "1m".into()),
        ("rsi_period", 2.into()),
        ("notional_usd", "100".into()),
        ("max_position", "10".into()),
        ("daily_loss_usd", "50".into()),
        ("cooldown_candles", 0.into()),
        ("stop_loss_pct", "0.2".into()),
    ] {
        table.entry(key).or_insert(value);
    }
    let config: RsiMomentumConfig = table.try_into().unwrap();
    config.validate().unwrap();
    config
}

fn book(mid: Decimal) -> OrderBook {
    let half = d("0.05");
    OrderBook {
        bids: BTreeMap::from([(mid - half, d("1000"))]),
        asks: BTreeMap::from([(mid + half, d("1000"))]),
        last_update_id: 0,
    }
}

fn long_pos(size: &str, entry: &str) -> Position {
    Position {
        symbol: SYM.into(),
        side: Some(Side::Buy),
        size: d(size),
        entry_price: d(entry),
        unrealized_pnl: Decimal::ZERO,
    }
}

#[path = "adapter_venue_tests.rs"]
mod venue_tests;

/// The alert goes on attempt `CLOSE_ATTEMPTS` only; once the venue reads flat, the next close alerts again.
#[test]
fn a_failing_close_alerts_once_per_close() {
    use bb_strategy_sdk::{Alert, Event, Intent, MarketFacts, StrategyAdapter, View};

    use crate::adapter::ALERT_CLOSE;
    use crate::limits::CLOSE_ATTEMPTS;

    let mut a = RsiMomentum::new(config(""));
    let held = [long_pos("1", "100")];
    let ob = book(d("79"));
    let view = |at: u64, position: &'static str, positions: &'static [Position]| View {
        now_ms: T0 + at * 1_000,
        symbol: SYM,
        book: Some(&ob),
        position: d(position),
        avg_entry: Some(d("100")),
        realized_pnl: Decimal::ZERO,
        open_orders: None,
        positions: Some(positions),
        reconnected: false,
        venue_halt: None,
        path_stop: None,
        read_started_ms: None,
        limits: None,
        market: MarketFacts { step_size: Some(d("0.001")), ..MarketFacts::default() },
        legs: &[],
    };
    let held: &'static [Position] = Box::leak(Box::new(held));
    let raised = |out: &[Intent]| {
        out.iter()
            .filter(|i| matches!(i, Intent::Notify(Alert::Raise { key, .. }) if key == ALERT_CLOSE))
            .count()
    };
    let closes = |out: &[Intent]| {
        out.iter().filter(|i| matches!(i, Intent::Place(o) if o.reduce_only)).count()
    };
    a.start(&view(0, "1", held)).unwrap();

    let mut at = 1;
    for close in 0..2 {
        // The mark is past the 20% stop under the 100 entry: attempt 1.
        let out = a.on_event(&Event::Mark(d("79")), &view(at, "1", held));
        assert_eq!((closes(&out), raised(&out)), (1, 0), "close {close}: the stop's first attempt");
        for attempt in 2..=CLOSE_ATTEMPTS + 3 {
            at += 1;
            let out = a.on_event(&Event::Tick, &view(at, "1", held));
            assert_eq!(closes(&out), 1, "close {close}, attempt {attempt}: retried every tick");
            let want = usize::from(attempt == CLOSE_ATTEMPTS);
            assert_eq!(raised(&out), want, "close {close}, attempt {attempt}");
        }
        at += 1;
        let out = a.on_event(&Event::Tick, &view(at, "0", &[]));
        assert!(out.is_empty(), "close {close}: the venue reads flat, the close ends: {out:?}");
        at += 1;
    }
}

/// The briefing says how the warm-up went while the RSI is still warming up, and stops once it
/// reads.
/// reads.
#[test]
fn the_briefing_names_a_live_warm_up_until_the_rsi_reads() {
    use bb_strategy_sdk::StrategyReport;
    let report = |closes_to_warm| crate::RsiReport {
        symbol: SYM.into(),
        interval: "1m".into(),
        rsi: None,
        closes_to_warm,
        mark: None,
        cooldown_candles_left: 0,
        close_attempts: None,
        close_failing: false,
        history: Some(
            "It couldn't load past prices (HTTP 451), so it warms up from live data. ".into(),
        ),
    };
    assert_eq!(report(3).briefing_lines(), [report(3).history.unwrap()]);
    assert_eq!(report(0).briefing_lines(), Vec::<String>::new());
}

/// It never displaces the book or entry price a close is priced off, nor hides the last good mark from the stop.
#[test]
fn a_non_positive_mark_never_blocks_a_close() {
    use bb_strategy_sdk::{Event, Intent, MarketFacts, StrategyAdapter, View};

    let ob: &'static OrderBook = Box::leak(Box::new(book(d("79"))));
    let held: &'static [Position] = Box::leak(Box::new([long_pos("1", "100")]));
    let view = |at: u64, book: Option<&'static OrderBook>, long: bool| View {
        now_ms: T0 + at * 1_000,
        symbol: SYM,
        book,
        position: if long { d("1") } else { Decimal::ZERO },
        avg_entry: long.then(|| d("100")),
        realized_pnl: Decimal::ZERO,
        open_orders: None,
        positions: Some(if long { held } else { &[] }),
        reconnected: false,
        venue_halt: None,
        path_stop: None,
        read_started_ms: None,
        limits: None,
        market: MarketFacts { step_size: Some(d("0.001")), ..MarketFacts::default() },
        legs: &[],
    };
    // A reduce-only sell priced no lower than `floor` (2% under the book's mid, or the entry).
    let closes = |out: &[Intent], floor: &str| {
        out.iter()
            .filter(|i| matches!(i, Intent::Place(o) if o.reduce_only && o.price >= d(floor)))
            .count()
    };
    for bad in ["0", "-1"] {
        let mut a = RsiMomentum::new(config(""));
        a.start(&view(0, Some(ob), true)).unwrap();
        let out = a.on_event(&Event::Mark(d("79")), &view(1, Some(ob), true));
        assert_eq!(closes(&out, "77.42"), 1, "the stop's first attempt");
        a.on_event(&Event::Mark(d(bad)), &view(2, Some(ob), true));
        let out = a.on_event(&Event::Tick, &view(3, Some(ob), true));
        assert_eq!(closes(&out, "77.42"), 1, "mark {bad}: the retry still goes out: {out:?}");
        let out = a.stop(&view(4, Some(ob), true));
        assert_eq!(closes(&out, "77.42"), 1, "mark {bad}: so does the wind-down's close");

        let mut a = RsiMomentum::new(config(""));
        a.start(&view(0, None, true)).unwrap();
        a.on_event(&Event::Mark(d(bad)), &view(1, None, true));
        let out = a.stop(&view(2, None, true));
        assert_eq!(closes(&out, "98"), 1, "mark {bad}: no book, the close is priced off the entry");

        let mut a = RsiMomentum::new(config(""));
        a.start(&view(0, Some(ob), false)).unwrap();
        a.on_event(&Event::Mark(d("79")), &view(1, Some(ob), false));
        a.on_event(&Event::Mark(d(bad)), &view(2, Some(ob), false));
        let out = a.on_event(&Event::Tick, &view(3, Some(ob), true));
        assert_eq!(closes(&out, "77.42"), 1, "mark {bad}: the stop still sees the 79 mark");
    }
}
