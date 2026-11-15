//! A [`StrategyAdapter`] with its report erased, so a host holds any strategy as one type
//! (`Box<dyn ErasedAdapter>`) and a caller names none. Every [`StrategyAdapter`] is one.

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use crate::history::candles::{HistoryNeed, Warmup};
use crate::strategy::adapter::{
    Event, Intent, OwnBook, SnapshotExtras, StartMode, StateRead, StrategyAdapter, StrategyError,
    StrategyReport, View,
};
use crate::strategy::twin::{TwinAdapter, TwinSpec};
use crate::types::Protection;

/// [`StrategyAdapter`] with its report erased: each method is the trait's.
pub trait ErasedAdapter: Send {
    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError>;
    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent>;
    fn stop(&mut self, view: &View<'_>) -> Vec<Intent>;
    fn report(&self) -> Arc<dyn StrategyReport>;
    fn tick_interval(&self) -> Option<Duration>;
    fn wants_venue(&self, now_ms: u64, reconnected: bool) -> bool;
    fn orders_cancelled(&mut self);
    /// # Errors
    /// As [`StrategyAdapter::twin`].
    fn twin(&self, spec: &TwinSpec) -> Result<Option<TwinAdapter>, String>;
    fn start_mode(&self) -> StartMode;
    fn own_book(&self) -> Option<OwnBook>;
    fn snapshot_extras(&self) -> SnapshotExtras;
    fn status_keys(&self) -> serde_json::Map<String, serde_json::Value>;
    /// The adapter as its own type, for a test's typed borrow.
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn faulted(&self) -> bool;
    fn history(&self) -> Option<HistoryNeed>;
    fn warm_up(&mut self, warmup: Warmup);
    fn protective_stop(&self, view: &View<'_>) -> Option<Protection>;
    fn reclaim(&mut self);
    fn restore(&mut self, state: &dyn StateRead) -> Result<(), StrategyError>;
    fn may_keep(&self) -> bool;
}

impl<S: StrategyAdapter> ErasedAdapter for S {
    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        StrategyAdapter::start(self, view)
    }

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent> {
        StrategyAdapter::on_event(self, event, view)
    }

    fn stop(&mut self, view: &View<'_>) -> Vec<Intent> {
        StrategyAdapter::stop(self, view)
    }

    fn report(&self) -> Arc<dyn StrategyReport> {
        Arc::new(StrategyAdapter::report(self))
    }

    fn tick_interval(&self) -> Option<Duration> {
        StrategyAdapter::tick_interval(self)
    }

    fn wants_venue(&self, now_ms: u64, reconnected: bool) -> bool {
        StrategyAdapter::wants_venue(self, now_ms, reconnected)
    }

    fn orders_cancelled(&mut self) {
        StrategyAdapter::orders_cancelled(self);
    }

    fn twin(&self, spec: &TwinSpec) -> Result<Option<TwinAdapter>, String> {
        StrategyAdapter::twin(self, spec)
    }

    fn start_mode(&self) -> StartMode {
        StrategyAdapter::start_mode(self)
    }

    fn own_book(&self) -> Option<OwnBook> {
        StrategyAdapter::own_book(self)
    }

    fn snapshot_extras(&self) -> SnapshotExtras {
        StrategyAdapter::snapshot_extras(self)
    }

    fn status_keys(&self) -> serde_json::Map<String, serde_json::Value> {
        StrategyAdapter::status_keys(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn faulted(&self) -> bool {
        StrategyAdapter::faulted(self)
    }

    fn history(&self) -> Option<HistoryNeed> {
        StrategyAdapter::history(self)
    }

    fn warm_up(&mut self, warmup: Warmup) {
        StrategyAdapter::warm_up(self, warmup);
    }

    fn protective_stop(&self, view: &View<'_>) -> Option<Protection> {
        StrategyAdapter::protective_stop(self, view)
    }

    fn reclaim(&mut self) {
        StrategyAdapter::reclaim(self);
    }

    fn restore(&mut self, state: &dyn StateRead) -> Result<(), StrategyError> {
        StrategyAdapter::restore(self, state)
    }

    fn may_keep(&self) -> bool {
        StrategyAdapter::may_keep(self)
    }
}
