//! The component side of [`export_strategy!`](crate::export_strategy): the strategy a component
//! runs, built by `init` and held for the component's life, and the calls of the `bb:strategy`
//! world's `adapter` interface over it. A component runs one strategy (one wasmtime `Store` per
//! instance on the host), single-threaded, so the strategy lives in a thread-local.

use std::cell::RefCell;

use crate::adapter::{FromSettings, StrategyAdapter, StrategyError, StrategyReport};
use crate::describe::Describe;
use crate::wire;
/// The world's generated types, for the macro.
pub use crate::wire::w;

/// The strategy behind the component's exports, with its report type erased.
trait Running {
    fn start(&mut self, view: &w::View) -> Result<Vec<w::Intent>, String>;
    fn on_event(&mut self, event: w::Event, view: &w::View) -> Vec<w::Intent>;
    fn stop(&mut self, view: &w::View) -> Vec<w::Intent>;
    fn report(&self) -> w::ReportRecord;
}

impl<S: StrategyAdapter> Running for S {
    fn start(&mut self, view: &w::View) -> Result<Vec<w::Intent>, String> {
        let owned = wire::OwnedView::from_wire(view).map_err(|e| e.to_string())?;
        let intents = StrategyAdapter::start(self, &owned.view()).map_err(|e| e.0)?;
        Ok(intents.into_iter().filter_map(wire::intent_to_wire).collect())
    }

    fn on_event(&mut self, event: w::Event, view: &w::View) -> Vec<w::Intent> {
        // The host encodes only valid decimals; a view or event it could not have sent asks for
        // nothing rather than deciding on a guess.
        let (Ok(owned), Ok(event)) =
            (wire::OwnedView::from_wire(view), wire::event_from_wire(event))
        else {
            return Vec::new();
        };
        let intents = StrategyAdapter::on_event(self, &event, &owned.view());
        intents.into_iter().filter_map(wire::intent_to_wire).collect()
    }

    fn stop(&mut self, view: &w::View) -> Vec<w::Intent> {
        let Ok(owned) = wire::OwnedView::from_wire(view) else { return Vec::new() };
        let intents = StrategyAdapter::stop(self, &owned.view());
        intents.into_iter().filter_map(wire::intent_to_wire).collect()
    }

    fn report(&self) -> w::ReportRecord {
        report_to_wire(&StrategyAdapter::report(self))
    }
}

/// A report as it crosses: its fields with what a generic reader asks of it.
pub(crate) fn report_to_wire(report: &impl StrategyReport) -> w::ReportRecord {
    w::ReportRecord {
        fields: report.fields().into_iter().map(wire::field_to_wire).collect(),
        headline: report.headline(),
        briefing_lines: report.briefing_lines(),
        paper_inputs: report.paper_inputs().map(|p| w::PaperInputs {
            order_size: wire::dec(p.order_size),
            quoting_secs: p.quoting_secs,
            maker_fee_bps: wire::dec(p.maker_fee_bps),
        }),
        reference_price: report.reference_price().map(wire::dec),
    }
}

/// The bytes of the `bb-strategy-meta` custom section: `name`, a newline, `version` (a name has
/// no newline).
#[doc(hidden)]
#[must_use]
#[expect(
    clippy::arithmetic_side_effects,
    reason = "const fn on short text; the length is asserted"
)]
pub const fn meta<const N: usize>(name: &str, version: &str) -> [u8; N] {
    let (name, version) = (name.as_bytes(), version.as_bytes());
    assert!(N == name.len() + 1 + version.len(), "the meta section has the wrong length");
    let mut out = [0_u8; N];
    let mut i = 0;
    while i < name.len() {
        out[i] = name[i];
        i += 1;
    }
    out[i] = b'\n';
    let mut j = 0;
    while j < version.len() {
        out[i + 1 + j] = version[j];
        j += 1;
    }
    out
}

thread_local! {
    static STRATEGY: RefCell<Option<Box<dyn Running>>> = const { RefCell::new(None) };
}

const NOT_STARTED: &str = "the strategy was not initialised (init first)";

/// `describe`: what `S` is, before any `init`.
#[must_use]
pub fn describe<S: Describe>() -> w::Descriptor {
    wire::descriptor_to_wire(S::NAME, S::VERSION, S::describe())
}

/// `init`: build `S` from its settings; a second `init` replaces the first.
///
/// # Errors
/// What `S::from_settings` refused.
pub fn init<S: StrategyAdapter + FromSettings>(settings: &str) -> Result<(), String> {
    let strategy = S::from_settings(settings).map_err(|StrategyError(e)| e)?;
    STRATEGY.with(|s| *s.borrow_mut() = Some(Box::new(strategy)));
    Ok(())
}

/// # Errors
/// The strategy's refusal, or that it was not initialised.
pub fn start(view: &w::View) -> Result<Vec<w::Intent>, String> {
    STRATEGY.with(|s| match s.borrow_mut().as_mut() {
        Some(strategy) => strategy.start(view),
        None => Err(NOT_STARTED.into()),
    })
}

#[must_use]
pub fn on_event(event: w::Event, view: &w::View) -> Vec<w::Intent> {
    STRATEGY.with(|s| s.borrow_mut().as_mut().map(|s| s.on_event(event, view)).unwrap_or_default())
}

#[must_use]
pub fn stop(view: &w::View) -> Vec<w::Intent> {
    STRATEGY.with(|s| s.borrow_mut().as_mut().map(|s| s.stop(view)).unwrap_or_default())
}

#[must_use]
pub fn report() -> w::ReportRecord {
    let none = || w::ReportRecord {
        fields: Vec::new(),
        headline: NOT_STARTED.into(),
        briefing_lines: Vec::new(),
        paper_inputs: None,
        reference_price: None,
    };
    STRATEGY.with(|s| s.borrow().as_ref().map_or_else(none, |s| s.report()))
}

#[cfg(test)]
#[path = "guest_tests.rs"]
mod tests;
