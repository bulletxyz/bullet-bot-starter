//! Parity of the funding arb's `decide` with the built-in's. Its fixtures
//! (`fixtures/funding-arb/*.jsonl`) hold, one line each, a view the built-in's own tests give
//! `decide` and the targets the real `decide` returned. The funding arb is not a strategy the bot's
//! host runs call by call (the bot's safety executor builds the view and carries out the targets),
//! so the unit compared is `decide` itself: the port is replayed on the same view and must return
//! the same targets, field for field.

use std::path::Path;

use serde_json::Value;

/// What one fixture's replay found.
#[derive(Debug, Default)]
pub struct DecideReport {
    /// The fixture's name (the built-in's test).
    pub test: String,
    /// Views replayed.
    pub views: usize,
    /// Targets compared equal.
    pub targets: usize,
    /// Each difference, in plain words.
    pub mismatches: Vec<String>,
}

/// Replay the fixture at `path`: `decide` gets each line's `view` (JSON) and returns its targets
/// (JSON); they must equal the line's `targets`.
///
/// # Errors
/// The fixture cannot be read or parsed (a difference is in the report, not an error).
pub fn replay<F>(path: &Path, decide: F) -> Result<DecideReport, String>
where
    F: Fn(&Value) -> Result<Value, String>,
{
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut report = DecideReport::default();
    for (n, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let line: Value = serde_json::from_str(line)
            .map_err(|e| format!("{}: line {}: {e}", path.display(), n.saturating_add(1)))?;
        match line["kind"].as_str() {
            Some("header") => {
                line["test"].as_str().unwrap_or_default().clone_into(&mut report.test);
            }
            Some("decide") => {
                report.views = report.views.saturating_add(1);
                let want = &line["targets"];
                match decide(&line["view"]) {
                    Ok(got) if got == *want => {
                        report.targets =
                            report.targets.saturating_add(want.as_array().map_or(0, Vec::len));
                    }
                    Ok(got) => report.mismatches.push(format!(
                        "line {}: the built-in returned {want}, the port {got}, for {}",
                        n.saturating_add(1),
                        line["view"]
                    )),
                    Err(e) => report
                        .mismatches
                        .push(format!("line {}: the port failed: {e}", n.saturating_add(1))),
                }
            }
            other => return Err(format!("line {}: unknown kind {other:?}", n.saturating_add(1))),
        }
    }
    Ok(report)
}

/// Replay every fixture in `dir`; fail with every difference. Returns the reports.
///
/// # Errors
/// A fixture that cannot be read, no fixtures, or any difference.
pub fn replay_all<F>(dir: &Path, decide: F) -> Result<Vec<DecideReport>, String>
where
    F: Fn(&Value) -> Result<Value, String>,
{
    let files = crate::fixtures(dir)?;
    if files.is_empty() {
        return Err(format!("no fixtures in {}", dir.display()));
    }
    let mut reports = Vec::new();
    let mut failures = Vec::new();
    for file in files {
        let report = replay(&file, &decide)?;
        failures.extend(report.mismatches.iter().map(|m| format!("{}: {m}", report.test)));
        reports.push(report);
    }
    if failures.is_empty() { Ok(reports) } else { Err(failures.join("\n")) }
}
