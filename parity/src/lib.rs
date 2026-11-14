//! Parity with bullet-bots' built-in strategies. A fixture (`fixtures/<strategy>/<test>.jsonl`) is
//! every call one of the built-in's own tests made to it under the bot's real `StrategyHost`: the
//! settings, then each `start`, event and `stop` with the view it was shown and the intents it
//! returned. [`replay`] makes the same calls on a port and compares what it returns.
//!
//! Each line is one JSON object (`kind`: `header`, `warm_up`, `start`, `event`, `stop`); events and
//! intents are bb-strategy's own JSON (serde, externally tagged), which is the SDK's for every case
//! the `bb:strategy` world has. What the bot does differently for a plugin is done here as its WASM
//! host does it:
//!
//! - an event the world has no case for (`Reference`, `Control`, `Persisted`, `Closed`) is not
//!   delivered; `Failed` arrives as a refused `Outcome` ("risk gate: new orders are stopped: ...");
//! - a plugin has no `CancelOwn` (its `CancelAll` takes only its own orders) and no `Notify`, so the
//!   built-in's `CancelOwn` is compared as `CancelAll` and its `Notify` is left out ([`Report`]
//!   counts both); a bot refuses a plugin's `Shutdown`, so a port stops itself and pulls its
//!   orders: the built-in's `Shutdown` is compared as `CancelAll`, and the replay ends there;
//! - client ids are the port's own: the first id the built-in used in an intent maps to the one the
//!   port used in the same place, and every later event and view carries the port's.
//!
//! A fixture is skipped when its built-in was given what a plugin never gets: past candles (a
//! `warm_up` that loaded, or `history`), hard limits (`limits`) or a daily loss probe (`day_stop`).
//! When the port refuses the built-in's settings, the built-in must have refused to start too.

use std::collections::BTreeMap;
use std::path::Path;

use bb_strategy_sdk::testing::Market;
use bb_strategy_sdk::{
    Event, Intent, MarketFacts, Order, OrderBook, Position, StrategyAdapter, StrategyError,
    StrategyReport,
};
use serde::Deserialize;
use serde_json::{Value, json};

/// What the WASM host says for a place the bot did not send (`bb_strategy::STOPPED_REFUSAL`).
pub const STOPPED_REFUSAL: &str = "new orders are stopped";

/// A view as the fixture holds it (bb-strategy's `View`, owned).
#[derive(Debug, Clone, Deserialize)]
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
    legs: Vec<Value>,
}

impl ViewJson {
    fn market(self) -> Result<Market, String> {
        if !self.legs.is_empty() {
            return Err("a view with legs: not in these fixtures".into());
        }
        Ok(Market {
            now_ms: self.now_ms,
            symbol: self.symbol,
            book: self.book,
            position: self.position,
            avg_entry: self.avg_entry,
            realized_pnl: self.realized_pnl,
            open_orders: self.open_orders,
            positions: self.positions,
            reconnected: self.reconnected,
            venue_halt: self.venue_halt,
            market: self.market,
            legs: Vec::new(),
            path_stop: self.path_stop,
            read_started_ms: self.read_started_ms,
        })
    }
}

/// What one fixture's replay found.
#[derive(Debug, Default)]
pub struct Report {
    /// The fixture's name (the built-in's test).
    pub test: String,
    /// Calls made on the port and compared.
    pub calls: usize,
    /// Intents compared equal.
    pub intents: usize,
    /// Events the world does not deliver, skipped.
    pub undelivered: usize,
    /// The built-in's `Notify` intents left out, and its `CancelOwn` compared as `CancelAll`.
    pub notify_dropped: usize,
    pub cancel_own: usize,
    /// The built-in asked the bot to stop (`Shutdown`), compared as the port's `CancelAll` (a
    /// plugin stops itself): the replay ends there, as the bot would.
    pub shutdown: bool,
    /// Headlines compared equal.
    pub headlines: usize,
    /// The port refused the settings when loaded, and the built-in refused to start on them.
    pub refused: bool,
    /// Why the fixture was not replayed, when it was not.
    pub skipped: Option<String>,
    /// Each difference, in plain words.
    pub mismatches: Vec<String>,
}

/// The fixture files under `dir`, sorted.
///
/// # Errors
/// The directory cannot be read.
pub fn fixtures(dir: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let read = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files: Vec<_> = read
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    Ok(files)
}

/// Client ids: the built-in's to the port's.
#[derive(Debug, Default)]
struct Ids(BTreeMap<String, String>);

impl Ids {
    /// `value` with every `client_id` the map knows replaced by the port's.
    fn to_port(&self, value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, v) in map.iter_mut() {
                    if key == "client_id"
                        && let Value::String(id) = v
                        && let Some(port) = self.0.get(id.as_str())
                    {
                        port.clone_into(id);
                    } else {
                        self.to_port(v);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|v| self.to_port(v)),
            _ => {}
        }
    }

    /// Learn the pairs `native` and `port` hold in the same places (both one intent's JSON).
    fn learn(&mut self, native: &Value, port: &Value) {
        match (native, port) {
            (Value::Object(a), Value::Object(b)) => {
                for (key, va) in a {
                    let Some(vb) = b.get(key) else { continue };
                    if key == "client_id"
                        && let (Value::String(x), Value::String(y)) = (va, vb)
                    {
                        self.0.entry(x.clone()).or_insert_with(|| y.clone());
                    } else {
                        self.learn(va, vb);
                    }
                }
            }
            (Value::Array(a), Value::Array(b)) => {
                a.iter().zip(b).for_each(|(x, y)| self.learn(x, y));
            }
            _ => {}
        }
    }
}

/// The built-in's intents as a plugin would send them: `Notify` left out, `CancelOwn` as
/// `CancelAll`; counted in `report`.
fn as_plugin(intents: &[Value], report: &mut Report) -> Vec<Value> {
    let mut out = Vec::new();
    for intent in intents {
        if intent.get("Notify").is_some() {
            report.notify_dropped = report.notify_dropped.saturating_add(1);
        } else if intent == "CancelOwn" {
            report.cancel_own = report.cancel_own.saturating_add(1);
            out.push(json!("CancelAll"));
        } else if intent.get("Shutdown").is_some() {
            // A bot refuses a plugin's shutdown; a port stops itself and pulls its orders.
            report.shutdown = true;
            out.push(json!("CancelAll"));
        } else {
            out.push(intent.clone());
        }
    }
    out
}

/// The event as the WASM host delivers it to a plugin, or `None` when it does not.
fn as_delivered(event: &Value) -> Option<Value> {
    if let Some(failed) = event.get("Failed") {
        let intent = failed.get("intent")?.clone();
        let error = failed.get("error").and_then(Value::as_str).unwrap_or_default();
        let place =
            intent.get("Place").or_else(|| intent.get("Amend").and_then(|a| a.get("new_order")));
        let client_id = place
            .or_else(|| intent.get("Cancel"))
            .and_then(|o| o.get("client_id"))
            .cloned()
            .unwrap_or(Value::Null);
        let error = if place.is_some() {
            format!("risk gate: {STOPPED_REFUSAL}: {error}")
        } else {
            error.to_owned()
        };
        let result =
            json!({"order_id": null, "client_id": client_id, "success": false, "error": error});
        return answered_as_plugin(json!({"Outcome": {"intent": intent, "result": result}}));
    }
    for host_only in ["Reference", "Control", "Persisted", "Closed"] {
        if event.get(host_only).is_some() {
            return None;
        }
    }
    answered_as_plugin(event.clone())
}

/// An outcome (or unknown) of an intent as a plugin hears it: of `CancelAll` for the built-in's
/// `CancelOwn`; none for an intent only the host asks for (a plugin never sent one).
fn answered_as_plugin(mut event: Value) -> Option<Value> {
    for at in ["/Outcome/intent", "/Unknown/intent"] {
        if let Some(intent) = event.pointer_mut(at)
            && intent == "CancelOwn"
        {
            *intent = json!("CancelAll");
        }
    }
    if let Some(intent) =
        event.pointer("/Outcome/intent").or_else(|| event.pointer("/Unknown/intent"))
        && (intent.get("Notify").is_some()
            || ["Persist", "Reply", "ClosePosition", "SetLimits"]
                .iter()
                .any(|k| intent.get(k).is_some()))
    {
        return None;
    }
    Some(event)
}

fn to_json(intents: &[Intent]) -> Result<Vec<Value>, String> {
    intents.iter().map(|i| serde_json::to_value(i).map_err(|e| e.to_string())).collect()
}

/// Compare the built-in's `native` with the port's `port` for call `at`; learn their ids.
fn compare(at: &str, native: &[Value], port: &[Intent], ids: &mut Ids, report: &mut Report) {
    let native = as_plugin(native, report);
    let Ok(port) = to_json(port) else {
        report.mismatches.push(format!("{at}: the port's intents do not serialize"));
        return;
    };
    for (n, p) in native.iter().zip(&port) {
        ids.learn(n, p);
    }
    let mut want = native.clone();
    for v in &mut want {
        ids.to_port(v);
    }
    if want == port {
        report.intents = report.intents.saturating_add(want.len());
    } else {
        report.mismatches.push(format!(
            "{at}: the built-in returned {}, the port {}",
            Value::Array(want),
            Value::Array(port)
        ));
    }
}

fn field<'a>(line: &'a Value, name: &str, at: &str) -> Result<&'a Value, String> {
    line.get(name).ok_or_else(|| format!("{at}: no `{name}`"))
}

fn view_of(line: &Value, ids: &Ids, at: &str) -> Result<Market, String> {
    let mut view = field(line, "view", at)?.clone();
    ids.to_port(&mut view);
    let view: ViewJson = serde_json::from_value(view).map_err(|e| format!("{at}: view: {e}"))?;
    view.market().map_err(|e| format!("{at}: {e}"))
}

fn intents_of<'a>(line: &'a Value, at: &str) -> Result<&'a [Value], String> {
    field(line, "intents", at)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{at}: intents"))
}

fn check_headline<S: StrategyAdapter>(strategy: &S, line: &Value, at: &str, report: &mut Report) {
    let Some(want) = line.get("headline").and_then(Value::as_str) else {
        return;
    };
    let got = strategy.report().headline();
    if got == want {
        report.headlines = report.headlines.saturating_add(1);
    } else {
        report.mismatches.push(format!("{at}: headline {got:?}, the built-in's {want:?}"));
    }
}

/// Why a fixture cannot be replayed on a plugin, if it cannot: its built-in was given what a plugin
/// never gets.
fn skip_reason(lines: &[Value]) -> Option<&'static str> {
    if lines.iter().any(|l| l["kind"] == "warm_up" && l["loaded"] == true) {
        return Some("the built-in warmed up on past candles; a plugin gets none");
    }
    let header = lines.first()?;
    [
        ("history", "the built-in was given past candles; a plugin gets none"),
        ("limits", "the built-in was given hard limits; a plugin gets none"),
        ("day_stop", "the built-in was given a daily loss probe; a plugin gets none"),
    ]
    .into_iter()
    .find_map(|(key, why)| header.get(key).is_some().then_some(why))
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
    if let Some(why) = skip_reason(&lines) {
        report.skipped = Some(why.into());
        return Ok(report);
    }
    let settings = header["settings"].to_string();
    let mut strategy = match build(&settings) {
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
        match line["kind"].as_str() {
            Some("warm_up") => {}
            Some("start") => {
                let market = view_of(line, &ids, &at)?;
                report.calls = report.calls.saturating_add(1);
                match (strategy.start(&market.view()), line.get("error")) {
                    (Ok(got), None) => {
                        compare(&at, intents_of(line, &at)?, &got, &mut ids, &mut report);
                    }
                    (Err(_), Some(_)) => {}
                    (Ok(got), Some(want)) => report.mismatches.push(format!(
                        "{at}: the built-in refused to start ({want}); the port sent {got:?}"
                    )),
                    (Err(e), None) => {
                        report.mismatches.push(format!("{at}: the port refused to start: {e}"));
                        return Ok(report);
                    }
                }
                if report.shutdown {
                    return Ok(report);
                }
                check_headline(&strategy, line, &at, &mut report);
            }
            Some("event") => {
                let Some(mut event) = as_delivered(field(line, "event", &at)?) else {
                    report.undelivered = report.undelivered.saturating_add(1);
                    if !intents_of(line, &at)?.is_empty() {
                        report.mismatches.push(format!(
                            "{at}: the built-in answered an event a plugin never gets: {}",
                            line["intents"]
                        ));
                    }
                    continue;
                };
                ids.to_port(&mut event);
                let event: Event =
                    serde_json::from_value(event).map_err(|e| format!("{at}: event: {e}"))?;
                let market = view_of(line, &ids, &at)?;
                report.calls = report.calls.saturating_add(1);
                let got = strategy.on_event(&event, &market.view());
                compare(&at, intents_of(line, &at)?, &got, &mut ids, &mut report);
                if report.shutdown {
                    return Ok(report);
                }
                check_headline(&strategy, line, &at, &mut report);
            }
            Some("stop") => {
                let market = view_of(line, &ids, &at)?;
                report.calls = report.calls.saturating_add(1);
                let got = strategy.stop(&market.view());
                compare(&at, intents_of(line, &at)?, &got, &mut ids, &mut report);
                if report.shutdown {
                    return Ok(report);
                }
                check_headline(&strategy, line, &at, &mut report);
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
