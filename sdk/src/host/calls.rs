//! Venue calls a strategy asks the host to make for it ([`Intent::Request`](crate::Intent)): a
//! read, a write, or a wait on the bot's clock. The host carries each out between events, through
//! the named exchange's gate, bounded by a deadline, and answers it exactly once
//! ([`Event::Answer`](crate::Event)) under the request's id. The bot's host reads the same types.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::types::{
    AccountEquity, AmendOrder, CancelOrder, CancelResult, FundingSample, NewOrder, Order,
    OrderBook, OrderResult, Position,
};

/// A read of the exchange's venue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadQuery {
    Positions,
    /// The orders resting on `symbol`: what frees a level whose order never landed.
    OpenOrders {
        symbol: String,
    },
    /// A fresh book, `depth` levels.
    Book {
        symbol: String,
        depth: usize,
    },
    AccountEquity,
    MarginRatio,
    /// Settled funding from `start_ms` (unix ms) on.
    FundingHistory {
        symbol: String,
        start_ms: i64,
    },
    OpenInterest {
        symbol: String,
    },
    /// Re-read the venue's metadata (fees, steps); the new facts show in the next view.
    RefreshMetadata,
}

/// What a request asks of the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VenueCall {
    Read(ReadQuery),
    /// New orders, as one batch on the exchange's gate.
    Place(Vec<NewOrder>),
    Cancel(Vec<CancelOrder>),
    Amend(Vec<AmendOrder>),
    CancelAll {
        symbol: String,
    },
    /// Wake at `until_ms` on the bot's clock.
    Wait {
        until_ms: u64,
    },
}

/// What a call came back with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Answer {
    Positions(Vec<Position>),
    OpenOrders(Vec<Order>),
    Book(OrderBook),
    AccountEquity(Option<AccountEquity>),
    MarginRatio(Option<Decimal>),
    FundingHistory(Vec<FundingSample>),
    OpenInterest(Option<Decimal>),
    /// One result per order, in the batch's order; a refusal before the venue does not succeed.
    Placed(Vec<OrderResult>),
    Cancelled(Vec<CancelResult>),
    /// A cancel all, a metadata refresh or a wait is done.
    Done,
}

/// A call that failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallError {
    pub message: String,
    /// It never reached the venue (the host refused it); otherwise a write may have landed.
    pub never_sent: bool,
    /// The venue says trying again may succeed.
    pub retryable: bool,
    /// The host's own error behind it, kept as the host gave it ([`Cause`]); never serialized.
    #[serde(skip)]
    pub cause: Option<Cause>,
}

impl CallError {
    /// An error with no host cause behind it.
    #[must_use]
    pub fn new(message: impl Into<String>, never_sent: bool, retryable: bool) -> Self {
        Self { message: message.into(), never_sent, retryable, cause: None }
    }

    /// This error with the host's own error behind it.
    #[must_use]
    pub fn caused_by(mut self, cause: Cause) -> Self {
        self.cause = Some(cause);
        self
    }
}

/// The host's own error behind a [`CallError`], opaque to a strategy: the host reads it back as the
/// type it made it from, so what it says about a call survives the trip through a strategy (a
/// retryable exchange error stays one). Never compared (two errors with the same words are equal),
/// never serialized.
#[derive(Clone)]
pub struct Cause(Arc<dyn Any + Send + Sync>);

impl Cause {
    #[must_use]
    pub fn new<T: Any + Send + Sync>(cause: T) -> Self {
        Self(Arc::new(cause))
    }

    /// The cause as a `T`, if it is one.
    #[must_use]
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }
}

impl PartialEq for Cause {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for Cause {}

impl fmt::Debug for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Cause")
    }
}
