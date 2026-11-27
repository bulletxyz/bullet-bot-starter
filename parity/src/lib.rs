//! A regression pin on bullet-bots' built-in strategies. A fixture
//! (`fixtures/<strategy>/<test>.jsonl`) is every call one of the built-in's own tests made to it
//! under the bot's real `StrategyHost`: the settings, then each `start`, event and `stop` with the
//! view it was shown and the intents it returned. [`replay`] makes the same calls on the built-in's
//! crate and compares what it returns.
//!
//! Each line is one JSON object (`kind`: `header`, `warm_up`, `start`, `event`, `stop`); events and
//! intents are bb-strategy's own JSON (serde, externally tagged). Client ids are the replay's own:
//! the first id the recording used in an intent maps to the one the replay used in the same place,
//! and every later event and view carries the replay's.
//!
//! When the port refuses the built-in's settings, the built-in must have refused to start too.

use std::collections::BTreeMap;
use std::path::Path;

use bb_strategy_sdk::testing::Market;
use bb_strategy_sdk::{Event, Intent, StrategyAdapter, StrategyError, StrategyReport};
use serde_json::Value;

mod shape;

/// What one fixture's replay found.
#[derive(Debug, Default)]
pub struct Report {
    /// The fixture's name (the built-in's test).
    pub test: String,
    /// Calls made on the port and compared.
    pub calls: usize,
    /// Intents compared equal.
    pub intents: usize,
    /// Headlines compared equal.
    pub headlines: usize,
    /// The port refused the settings when loaded, and the built-in refused to start on them.
    pub refused: bool,
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

fn to_json(intents: &[Intent]) -> Result<Vec<Value>, String> {
    intents.iter().map(|i| serde_json::to_value(i).map_err(|e| e.to_string())).collect()
}

/// Compare the built-in's `native` with the port's `port` for call `at`; learn their ids.
fn compare(at: &str, native: &[Value], port: &[Intent], ids: &mut Ids, report: &mut Report) {
    let Ok(port) = to_json(port) else {
        report.mismatches.push(format!("{at}: the port's intents do not serialize"));
        return;
    };
    for (n, p) in native.iter().zip(&port) {
        ids.learn(n, p);
    }
    let mut want = native.to_vec();
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

/// The call's view in the SDK's shape, and its main market's symbol.
fn view_of(line: &Value, ids: &Ids, at: &str) -> Result<(Market, String), String> {
    let mut view = field(line, "view", at)?.clone();
    ids.to_port(&mut view);
    let view: shape::ViewJson =
        serde_json::from_value(view).map_err(|e| format!("{at}: view: {e}"))?;
    let symbol = view.symbol.clone();
    Ok((view.market().map_err(|e| format!("{at}: {e}"))?, symbol))
}

/// The intents the built-in returned, in the SDK's shape.
fn intents_of(line: &Value, symbol: &str, at: &str) -> Result<Vec<Value>, String> {
    let intents = field(line, "intents", at)?.as_array().ok_or_else(|| format!("{at}: intents"))?;
    Ok(intents.iter().map(|i| shape::intent(i, symbol)).collect())
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
                let (market, symbol) = view_of(line, &ids, &at)?;
                report.calls = report.calls.saturating_add(1);
                match (strategy.start(&market.view()), line.get("error")) {
                    (Ok(got), None) => {
                        compare(&at, &intents_of(line, &symbol, &at)?, &got, &mut ids, &mut report);
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
                check_headline(&strategy, line, &at, &mut report);
            }
            Some("event") => {
                let (market, symbol) = view_of(line, &ids, &at)?;
                let mut event = shape::event(field(line, "event", &at)?, &symbol);
                ids.to_port(&mut event);
                let event: Event =
                    serde_json::from_value(event).map_err(|e| format!("{at}: event: {e}"))?;
                report.calls = report.calls.saturating_add(1);
                let got = strategy.on_event(&event, &market.view());
                compare(&at, &intents_of(line, &symbol, &at)?, &got, &mut ids, &mut report);
                check_headline(&strategy, line, &at, &mut report);
            }
            Some("stop") => {
                let (market, symbol) = view_of(line, &ids, &at)?;
                report.calls = report.calls.saturating_add(1);
                let got = strategy.stop(&market.view());
                compare(&at, &intents_of(line, &symbol, &at)?, &got, &mut ids, &mut report);
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
