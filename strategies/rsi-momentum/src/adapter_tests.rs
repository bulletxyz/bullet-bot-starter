//! The strategy alone, over hand-built views (the host's tests, with a mock venue, are the bot's).

#![expect(clippy::arithmetic_side_effects, reason = "test arithmetic")]

use std::collections::BTreeMap;

use bb_strategy_sdk::testing::{Market, on_bullet};
use bb_strategy_sdk::{
    ExchangeIntent, Intent, MarketEvent, NewOrder, OrderBook, Position, Side, ValidateConfig,
};
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

/// Its market on Bullet at `now_ms` with a 0.001 size step: `book`, the host's `position` and
/// `avg_entry`, and the venue's `positions` as read (`None`: not read this time).
fn shown(
    now_ms: u64,
    book: Option<OrderBook>,
    position: Decimal,
    avg_entry: Option<Decimal>,
    positions: Option<Vec<Position>>,
) -> Market {
    let mut shown = Market::new(SYM, "0", "0", now_ms);
    shown.set_book(book);
    let market = shown.main_mut().unwrap();
    market.position = position;
    market.avg_entry = avg_entry;
    market.facts.step_size = Some(d("0.001"));
    let bullet = shown.bullet_mut().unwrap();
    bullet.positions = positions;
    bullet.open_orders = None;
    shown
}

/// Bullet's mark for its market.
fn mark(price: &str) -> bb_strategy_sdk::Event {
    on_bullet(SYM, MarketEvent::Mark(d(price)))
}

/// The place `intent` asks for on Bullet, if it is one.
fn place(intent: &Intent) -> Option<&NewOrder> {
    match intent {
        Intent::On { exchange, intent: ExchangeIntent::Place(o) }
            if exchange == bb_strategy_sdk::BULLET =>
        {
            Some(o)
        }
        _ => None,
    }
}

#[path = "adapter_venue_tests.rs"]
mod venue_tests;

/// rsi-momentum trades its own symbol: a Bullet that shows another market is not a start.
#[test]
fn start_refuses_without_its_market_on_bullet() {
    let other = Market::new("ETH-USD", "99", "101", T0);
    let mut sim = bb_strategy_sdk::testing::SimHost::new(RsiMomentum::new(config("")), other);
    assert!(sim.start().unwrap_err().0.contains("no BTC-USD market"));
    assert_eq!(sim.sent, Vec::new(), "nothing sent");
}

/// The alert goes on attempt `CLOSE_ATTEMPTS` only; once the venue reads flat, the next close alerts again.
#[test]
fn a_failing_close_alerts_once_per_close() {
    use bb_strategy_sdk::{Alert, Event, StrategyAdapter};

    use crate::adapter::ALERT_CLOSE;
    use crate::limits::CLOSE_ATTEMPTS;

    let mut a = RsiMomentum::new(config(""));
    let held = [long_pos("1", "100")];
    let view = |at: u64, position: &str, positions: &[Position]| {
        let positions = Some(positions.to_vec());
        shown(T0 + at * 1_000, Some(book(d("79"))), d(position), Some(d("100")), positions)
    };
    let raised = |out: &[Intent]| {
        out.iter()
            .filter(|i| matches!(i, Intent::Notify(Alert::Raise { key, .. }) if key == ALERT_CLOSE))
            .count()
    };
    let closes = |out: &[Intent]| out.iter().filter_map(place).filter(|o| o.reduce_only).count();
    a.start(&view(0, "1", &held).view()).unwrap();

    let mut at = 1;
    for close in 0..2 {
        // The mark is past the 20% stop under the 100 entry: attempt 1.
        let out = a.on_event(&mark("79"), &view(at, "1", &held).view());
        assert_eq!((closes(&out), raised(&out)), (1, 0), "close {close}: the stop's first attempt");
        for attempt in 2..=CLOSE_ATTEMPTS + 3 {
            at += 1;
            let out = a.on_event(&Event::Tick, &view(at, "1", &held).view());
            assert_eq!(closes(&out), 1, "close {close}, attempt {attempt}: retried every tick");
            let want = usize::from(attempt == CLOSE_ATTEMPTS);
            assert_eq!(raised(&out), want, "close {close}, attempt {attempt}");
        }
        at += 1;
        let out = a.on_event(&Event::Tick, &view(at, "0", &[]).view());
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
    use bb_strategy_sdk::{Event, StrategyAdapter};

    let ob = book(d("79"));
    let view = |at: u64, book: Option<&OrderBook>, long: bool| {
        let position = if long { d("1") } else { Decimal::ZERO };
        let positions = if long { vec![long_pos("1", "100")] } else { Vec::new() };
        let entry = long.then(|| d("100"));
        shown(T0 + at * 1_000, book.cloned(), position, entry, Some(positions))
    };
    // A reduce-only sell priced no lower than `floor` (2% under the book's mid, or the entry).
    let closes = |out: &[Intent], floor: &str| {
        out.iter().filter_map(place).filter(|o| o.reduce_only && o.price >= d(floor)).count()
    };
    for bad in ["0", "-1"] {
        let mut a = RsiMomentum::new(config(""));
        a.start(&view(0, Some(&ob), true).view()).unwrap();
        let out = a.on_event(&mark("79"), &view(1, Some(&ob), true).view());
        assert_eq!(closes(&out, "77.42"), 1, "the stop's first attempt");
        a.on_event(&mark(bad), &view(2, Some(&ob), true).view());
        let out = a.on_event(&Event::Tick, &view(3, Some(&ob), true).view());
        assert_eq!(closes(&out, "77.42"), 1, "mark {bad}: the retry still goes out: {out:?}");
        let out = a.stop(&view(4, Some(&ob), true).view());
        assert_eq!(closes(&out, "77.42"), 1, "mark {bad}: so does the wind-down's close");

        let mut a = RsiMomentum::new(config(""));
        a.start(&view(0, None, true).view()).unwrap();
        a.on_event(&mark(bad), &view(1, None, true).view());
        let out = a.stop(&view(2, None, true).view());
        assert_eq!(closes(&out, "98"), 1, "mark {bad}: no book, the close is priced off the entry");

        let mut a = RsiMomentum::new(config(""));
        a.start(&view(0, Some(&ob), false).view()).unwrap();
        a.on_event(&mark("79"), &view(1, Some(&ob), false).view());
        a.on_event(&mark(bad), &view(2, Some(&ob), false).view());
        let out = a.on_event(&Event::Tick, &view(3, Some(&ob), true).view());
        assert_eq!(closes(&out, "77.42"), 1, "mark {bad}: the stop still sees the 79 mark");
    }
}
