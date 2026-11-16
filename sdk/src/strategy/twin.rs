//! The staging twin as a strategy sees it: what it is asked to be ([`TwinSpec`]), the history it
//! records into and reads ([`TwinHistory`]) and the instance it builds ([`TwinAdapter`]). The host
//! runs the twin beside the bot, dry, on the same market; a twin never sends an order, writes
//! state, alerts or answers a command. Which twins exist and who sees them is the host's.

use std::sync::Arc;

use crate::history::records::{FillRecord, HistoryEvent, HistoryReads, WindowMetrics};
use crate::host::control::SettingChanges;
use crate::strategy::adapter::StrategyAdapter;
use crate::strategy::erased::ErasedAdapter;

/// What a twin is asked to be: the running bot's settings with `changes` on top.
#[derive(Debug, Clone)]
pub struct TwinSpec {
    pub name: String,
    pub changes: SettingChanges,
    pub reason: String,
    /// The bot's clock when it was staged, Unix milliseconds.
    pub now_ms: u64,
    /// The twin's own history, in memory: what it records, and what its view is compared on.
    pub history: TwinHistory,
}

/// A twin's own fills, mids and P&L points (never written anywhere): where its records go
/// ([`Self::record`]) and what it reads back ([`Self::reads`]).
#[derive(Clone)]
pub struct TwinHistory {
    reads: Arc<dyn HistoryReads>,
    record: Arc<dyn Fn(&HistoryEvent) + Send + Sync>,
}

impl TwinHistory {
    /// A history that keeps nothing: reads find nothing, records are dropped.
    #[must_use]
    pub fn new() -> Self {
        Self::from_parts(Arc::new(NoHistory), Arc::new(|_| {}))
    }

    /// A history over the host's own store.
    #[must_use]
    pub fn from_parts(
        reads: Arc<dyn HistoryReads>,
        record: Arc<dyn Fn(&HistoryEvent) + Send + Sync>,
    ) -> Self {
        Self { reads, record }
    }

    /// What the twin reads back (its fill counts, its outcomes).
    #[must_use]
    pub fn reads(&self) -> Arc<dyn HistoryReads> {
        Arc::clone(&self.reads)
    }

    /// Where the twin's records go.
    #[must_use]
    pub fn record(&self) -> Arc<dyn Fn(&HistoryEvent) + Send + Sync> {
        Arc::clone(&self.record)
    }
}

impl Default for TwinHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TwinHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TwinHistory")
    }
}

/// The history that keeps nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHistory;

impl HistoryReads for NoHistory {
    fn fills(&self, _from_ms: u64, _to_ms: u64) -> Vec<FillRecord> {
        Vec::new()
    }

    fn window_metrics(&self, _from_ms: u64, _to_ms: u64) -> WindowMetrics {
        WindowMetrics::default()
    }
}

/// A strategy instance that runs as a twin. Made by [`StrategyAdapter::twin`].
pub struct TwinAdapter(Box<dyn ErasedAdapter>);

impl TwinAdapter {
    #[must_use]
    pub fn new(adapter: impl StrategyAdapter) -> Self {
        Self(Box::new(adapter))
    }

    /// The twin, for the host that runs it.
    #[must_use]
    pub fn into_inner(self) -> Box<dyn ErasedAdapter> {
        self.0
    }
}
