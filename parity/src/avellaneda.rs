//! Replay for the Avellaneda-Stoikov fixtures (`fixtures/avellaneda-stoikov/`). The built-in is an
//! actor on the bot's harness, not a strategy-host adapter, so these were recorded from scripted
//! scenarios driving the real actor on a scripted broker; the lines have the same shape as the
//! other fixtures (`header`, `start`, `event`, `stop`: the view, the event, the intents), with what
//! its broker calls amount to as intents (an open-orders read as an `Intent::Request` answered by
//! an `Answer` event line) and one `Outcome` event line per answered call.
//!
//! A view here may carry legs (the reference leg's book), which the common replay does not take:
//! `legs` is a list of `{id, venue, symbols, books}`. Everything else is as in [`crate::replay`]:
//! the first client id the built-in used maps to the port's, and the headline is not compared (the
//! actor has none).

use std::collections::BTreeMap;
use std::path::Path;

use bb_strategy_sdk::testing::Market;
use bb_strategy_sdk::{
    Event, LegView, MarketFacts, Order, OrderBook, Position, StrategyAdapter, StrategyError,
};
use serde::Deserialize;
use serde_json::Value;

use crate::{Ids, Report, compare, field, fixtures, intents_of};

#[derive(Debug, Deserialize)]
struct LegJson {
    id: String,
    venue: String,
    symbols: Vec<String>,
    books: BTreeMap<String, OrderBook>,
}

#[derive(Debug, Deserialize)]
struct ViewJson {
    now_ms: u64,
    symbol: String,
    book: Option<OrderBook>,
    position: bb_strategy_sdk::Decimal,
    avg_entry: Option<bb_strategy_sdk::Decimal>,
    realized_pnl: bb_strategy_sdk::Decimal,
    open_orders: Option<Vec<Order>>,
    positions: Option<Vec<Position>>,
    reconnected: bool,
    venue_halt: Option<String>,
    path_stop: Option<String>,
    read_started_ms: Option<u64>,
    market: MarketFacts,
    #[serde(default)]
    legs: Vec<LegJson>,
}

fn market_of(line: &Value, ids: &Ids, at: &str) -> Result<Market, String> {
    let mut view = field(line, "view", at)?.clone();
    ids.to_port(&mut view);
    let v: ViewJson = serde_json::from_value(view).map_err(|e| format!("{at}: view: {e}"))?;
    let legs = v
        .legs
        .into_iter()
        .map(|l| LegView { books: l.books, ..LegView::new(&l.id, &l.venue, l.symbols) })
        .collect();
    Ok(Market {
        now_ms: v.now_ms,
        symbol: v.symbol,
        book: v.book,
        position: v.position,
        avg_entry: v.avg_entry,
        realized_pnl: v.realized_pnl,
        open_orders: v.open_orders,
        positions: v.positions,
        reconnected: v.reconnected,
        venue_halt: v.venue_halt,
        market: v.market,
        legs,
        path_stop: v.path_stop,
        read_started_ms: v.read_started_ms,
    })
}

/// Replay the fixture at `path` through the port `build` makes from the fixture's settings (JSON).
///
/// # Errors
/// The fixture cannot be read or parsed (a difference is not an error: it is in the report).
pub fn replay<S, F>(path: &Path, build: F) -> Result<Report, String>
where
    S: StrategyAdapter,
    F: Fn(&str) -> Result<S, StrategyError>,
{
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let lines: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let header = lines.first().filter(|l| l["kind"] == "header").ok_or("no header line")?;
    let mut report = Report {
        test: header["test"].as_str().unwrap_or_default().to_owned(),
        ..Report::default()
    };
    let mut strategy = match build(&header["settings"].to_string()) {
        Ok(s) => s,
        Err(e) => {
            let start = lines.iter().find(|l| l["kind"] == "start");
            if start.is_some_and(|s| s.get("error").is_some()) {
                report.refused = true;
            } else {
                report.mismatches.push(format!("the port refused the built-in's settings: {e}"));
            }
            return Ok(report);
        }
    };
    let mut ids = Ids::default();
    for (n, line) in lines.iter().enumerate().skip(1) {
        let at = format!("line {}", n.saturating_add(1));
        let market = market_of(line, &ids, &at)?;
        report.calls = report.calls.saturating_add(1);
        match line["kind"].as_str() {
            Some("start") => match (strategy.start(&market.view()), line.get("error")) {
                (Ok(got), None) => {
                    compare(&at, intents_of(line, &at)?, &got, &mut ids, &mut report);
                }
                (Err(_), Some(_)) => report.refused = true,
                (Ok(got), Some(want)) => report.mismatches.push(format!(
                    "{at}: the built-in refused to start ({want}); the port sent {got:?}"
                )),
                (Err(e), None) => {
                    report.mismatches.push(format!("{at}: the port refused to start: {e}"));
                    return Ok(report);
                }
            },
            Some("event") => {
                let mut event = field(line, "event", &at)?.clone();
                ids.to_port(&mut event);
                let event: Event =
                    serde_json::from_value(event).map_err(|e| format!("{at}: event: {e}"))?;
                let got = strategy.on_event(&event, &market.view());
                compare(&at, intents_of(line, &at)?, &got, &mut ids, &mut report);
            }
            Some("stop") => {
                let got = strategy.stop(&market.view());
                compare(&at, intents_of(line, &at)?, &got, &mut ids, &mut report);
            }
            other => return Err(format!("{at}: unknown kind {other:?}")),
        }
    }
    Ok(report)
}

/// Replay every fixture in `dir` and fail with every difference found. Returns the reports.
///
/// # Errors
/// A fixture that cannot be read, or any difference, in plain words.
pub fn replay_all<S, F>(dir: &Path, build: F) -> Result<Vec<Report>, String>
where
    S: StrategyAdapter,
    F: Fn(&str) -> Result<S, StrategyError>,
{
    let files = fixtures(dir)?;
    if files.is_empty() {
        return Err(format!("no fixtures in {}", dir.display()));
    }
    let mut reports = Vec::new();
    let mut failures = Vec::new();
    for file in files {
        let report = replay(&file, &build)?;
        for m in &report.mismatches {
            failures.push(format!("{}: {m}", report.test));
        }
        reports.push(report);
    }
    if failures.is_empty() { Ok(reports) } else { Err(failures.join("\n")) }
}
