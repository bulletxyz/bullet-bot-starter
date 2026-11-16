//! The data of the host capabilities a strategy declares beyond orders: a price from another venue,
//! control commands and their answers, writes to the bot's state, alerts to the owner, and the
//! close of a position. Each is an [`Event`](crate::Event) the host delivers or an
//! [`Intent`](crate::Intent) the host carries out, so any strategy can use it and none is named
//! here.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// A price from a venue the strategy does not trade on (a fair value taken from another market), as
/// the host saw it. The host delivers it as [`Event::Reference`](crate::Event::Reference) for the
/// reference feed it was wired to, whatever the symbol says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferencePrice {
    /// The symbol as the source names it (e.g. `BTCUSDT`), not the host's.
    pub symbol: String,
    /// The source's mid (a microprice when the source weights by size).
    pub mid: Decimal,
    pub bid: Decimal,
    pub ask: Decimal,
    /// When the host received it (`DynClock::unix_ms`). A strategy judges staleness against
    /// [`View::now_ms`](crate::View::now_ms) with this, never against a clock of its own.
    pub received_ms: u64,
}

/// An alert for the owner, asked of the host's notifier
/// ([`Intent::Notify`](crate::Intent::Notify)). `key` is stable so the notifier can throttle
/// repeats and match a `Resolve` to its `Raise`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Alert {
    /// Raise (or re-raise) the alert `key`; the notifier throttles repeats.
    Raise { key: String, text: String },
    /// The condition behind `key` cleared.
    Resolve { key: String },
}

/// "Persist this": one write to the bot's state, asked of the host
/// ([`Intent::Persist`](crate::Intent::Persist)). The host's store
/// (the host's state writer) knows what each `key` means and
/// applies the `payload`; the strategy never sees a store.
///
/// Writes are applied in the order they were asked. With an `ack`, the host carries out nothing
/// else of that round (no order, no cancel, no reply) until the write is applied, then answers with
/// [`Event::Persisted`](crate::Event::Persisted) under that id; a write that fails comes back as
/// `Err`. Without one, nothing waits for it and a failure is only logged by the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Persist {
    pub key: String,
    pub payload: serde_json::Value,
    /// An id of the strategy's own to be told when the write is applied; `None` waits for nothing.
    pub ack: Option<u64>,
}

impl Persist {
    /// A write nothing waits for.
    #[must_use]
    pub fn new(key: impl Into<String>, payload: serde_json::Value) -> Self {
        Self { key: key.into(), payload, ack: None }
    }

    /// A write the host applies before anything else of the round, and answers under `id`.
    #[must_use]
    pub fn acked(key: impl Into<String>, payload: serde_json::Value, id: u64) -> Self {
        Self { key: key.into(), payload, ack: Some(id) }
    }
}

/// What the host's close of a position found and did
/// ([`Event::Closed`](crate::Event::Closed)). The strategy words its own answer from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseReport {
    /// Cancelling the symbol's orders failed (the error's text): nothing was closed, because
    /// something of the strategy's may still rest.
    pub cancel_error: Option<String>,
    /// The venue's signed position (long positive) when the close started, or the read's error.
    pub start: Result<Decimal, String>,
    /// The venue's signed position after the last attempt (equals `start` when none was made and
    /// it was read; zero when it could not be).
    pub remaining: Decimal,
    /// Reduce-only IOCs sent.
    pub attempts: u32,
    /// Why the close stopped before the position was flat, if it did.
    pub stopped_early: Option<String>,
    /// The client ids of the closing orders, so the strategy can tell their fills from its quotes.
    pub client_ids: Vec<String>,
    /// The venue order ids the closing orders got.
    pub order_ids: Vec<String>,
}

/// The hard limits a strategy has tightened live ([`Intent::SetLimits`](crate::Intent::SetLimits)),
/// as the strategy now holds them. The host applies a value only where it is tighter than the one
/// it holds (the order path's gate and the snapshot's limits), so no intent can loosen a limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitsUpdate {
    pub max_position: Decimal,
    pub max_order_size: Decimal,
    pub max_notional_usd: Option<Decimal>,
    pub max_orders_per_min: Option<u32>,
    pub daily_loss_usd: Option<Decimal>,
    pub stale_secs: Option<u64>,
}
