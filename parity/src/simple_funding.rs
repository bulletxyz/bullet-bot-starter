//! Parity for the simple-funding port. The built-in's fixtures (`fixtures/simple-funding/<test>.jsonl`)
//! are every call its own tests made to its pure decision functions: `decide` (the basket), `plan`
//! (the caps again, as orders), `assess` / `fillable` / `fills` (the book walk's cost and depth) and
//! `signal` (the median of settled funding). Each line holds the call's input and what the built-in
//! returned; [`replay_all`] gives the same input to the port's functions ([`Cores`]) and compares.
//!
//! The execution layer (reads, IOC orders, pair entries, unwinds, stops) is not replayed here: it
//! is a request/answer flow on the 0.5 world, tested in the port with the SDK's simulator. A byte
//! replay of the built-in's venue calls would only pin the call order, not the decisions.

use std::path::Path;

use bb_strategy_sdk::Decimal;
use serde_json::Value;

use crate::fixtures;

/// The port's decision functions over the fixture's JSON: each takes one fixture line and returns
/// the value the line's expected output is compared with (`decide`: the `targets` array, `plan`:
/// `actions`, `assess`: `{"round_trip", "depth_usd"}`, `fillable` / `fills`: a bool, `signal`:
/// a decimal string or null).
pub struct Cores<'a> {
    pub decide: &'a dyn Fn(&Value) -> Result<Value, String>,
    pub plan: &'a dyn Fn(&Value) -> Result<Value, String>,
    pub assess: &'a dyn Fn(&Value) -> Result<Value, String>,
    pub fillable: &'a dyn Fn(&Value) -> Result<Value, String>,
    pub fills: &'a dyn Fn(&Value) -> Result<Value, String>,
    pub signal: &'a dyn Fn(&Value) -> Result<Value, String>,
}

/// What one fixture's replay found.
#[derive(Debug, Default)]
pub struct Report {
    pub test: String,
    /// Calls compared equal, by kind: decide, plan, assess, fillable, fills, signal.
    pub calls: [usize; 6],
    pub mismatches: Vec<String>,
}

impl Report {
    #[must_use]
    pub fn total(&self) -> usize {
        self.calls.iter().sum()
    }
}

/// `value` with every decimal string written the same way (`0.10` and `0.1` compare equal).
fn normal(value: &Value) -> Value {
    match value {
        Value::String(s) => s
            .parse::<Decimal>()
            .map_or_else(|_| value.clone(), |d| Value::String(d.normalize().to_string())),
        Value::Array(items) => Value::Array(items.iter().map(normal).collect()),
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), normal(v))).collect())
        }
        other => other.clone(),
    }
}

fn expected(line: &Value, kind: &str) -> Value {
    match kind {
        "decide" => line["targets"].clone(),
        "plan" => line["actions"].clone(),
        "assess" => {
            serde_json::json!({"round_trip": line["round_trip"], "depth_usd": line["depth_usd"]})
        }
        "fillable" => line["fillable"].clone(),
        "fills" => line["fills"].clone(),
        _ => line["signal"].clone(),
    }
}

/// Replay one fixture through `cores`.
///
/// # Errors
/// The file cannot be read or parsed (a difference is in the report).
pub fn replay(path: &Path, cores: &Cores<'_>) -> Result<Report, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut report = Report::default();
    for (n, raw) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let line: Value =
            serde_json::from_str(raw).map_err(|e| format!("{}: {e}", path.display()))?;
        let kind = line["kind"].as_str().unwrap_or_default();
        if kind == "header" {
            line["test"].as_str().unwrap_or_default().clone_into(&mut report.test);
            continue;
        }
        let (slot, core) = match kind {
            "decide" => (0, cores.decide),
            "plan" => (1, cores.plan),
            "assess" => (2, cores.assess),
            "fillable" => (3, cores.fillable),
            "fills" => (4, cores.fills),
            "signal" => (5, cores.signal),
            other => return Err(format!("{}: line {n}: unknown kind {other:?}", path.display())),
        };
        let want = normal(&expected(&line, kind));
        match core(&line) {
            Ok(got) if normal(&got) == want => {
                if let Some(c) = report.calls.get_mut(slot) {
                    *c = c.saturating_add(1);
                }
            }
            Ok(got) => report.mismatches.push(format!(
                "line {n} ({kind}): the built-in returned {want}, the port {}",
                normal(&got)
            )),
            Err(e) => report.mismatches.push(format!("line {n} ({kind}): the port failed: {e}")),
        }
    }
    Ok(report)
}

/// Replay every fixture in `dir`.
///
/// # Errors
/// A fixture that cannot be read, or every difference found, in plain words.
pub fn replay_all(dir: &Path, cores: &Cores<'_>) -> Result<Vec<Report>, String> {
    let files = fixtures(dir)?;
    if files.is_empty() {
        return Err(format!("no fixtures in {}", dir.display()));
    }
    let mut reports = Vec::new();
    let mut failures = Vec::new();
    for file in files {
        let report = replay(&file, cores)?;
        failures.extend(report.mismatches.iter().map(|m| format!("{}: {m}", report.test)));
        reports.push(report);
    }
    if failures.is_empty() { Ok(reports) } else { Err(failures.join("\n")) }
}
