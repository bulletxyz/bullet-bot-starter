//! Venue calls a strategy asks the host to make for it ([`Intent::Request`](crate::Intent)): a
//! read, a write, or a wait on the bot's clock. The host carries each out between events, through
//! the named leg's gate, bounded by a deadline, and answers it exactly once
//! ([`Event::Answer`](crate::Event)) under the request's id. Same shape as bb-strategy's.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::types::{
    AccountEquity, AmendOrder, CancelOrder, CancelResult, FundingSample, NewOrder, Order,
    OrderBook, OrderResult, Position,
};

/// A read of the leg's venue.
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
    /// New orders, as one batch on the leg's gate.
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
}
