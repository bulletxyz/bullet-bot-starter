//! The plumbing between the strategy's async-shaped code and the world's request/answer calls.
//!
//! The strategy's flows are `async` code that awaits venue calls (`io.call(..)`). Each await files
//! an [`Intent::Request`](bb_strategy_sdk::Intent) with the next id and parks; the host answers it
//! with an `Event::Answer` and the flow resumes. One call is in flight at a time, so ids never
//! collide. Nothing here touches a clock or the network: time is the view's `now_ms`.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};

use bb_strategy_sdk::{Answer, CallError, MarketFacts, OrderBook, VenueCall};
use rust_decimal::Decimal;

/// A mark or a book and when it came (the bot's clock, ms).
#[derive(Debug, Clone, Default)]
pub struct Seen {
    pub mark: Option<Decimal>,
    pub mark_at_ms: Option<u64>,
    pub book: Option<OrderBook>,
    pub book_at_ms: Option<u64>,
}

/// What the host showed, and the calls waiting for it, shared by the flow and the adapter.
#[derive(Debug, Default)]
pub struct Shared {
    /// The bot's clock, Unix ms, as of the last event.
    pub now_ms: u64,
    pub seen: BTreeMap<String, Seen>,
    pub facts: BTreeMap<String, MarketFacts>,
    /// The host stops new orders (its gate's or the venue's reason), as of the last event.
    pub order_stop: Option<String>,
    /// What the last flow left for the report.
    pub status: crate::report::Status,
    next_id: u64,
    requests: Vec<(u64, String, VenueCall)>,
    answers: BTreeMap<u64, Result<Answer, CallError>>,
}

impl Shared {
    /// The calls filed since the last drain, in order.
    pub fn take_requests(&mut self) -> Vec<(u64, String, VenueCall)> {
        std::mem::take(&mut self.requests)
    }

    /// The host's answer to request `id`.
    pub fn answer(&mut self, id: u64, reply: Result<Answer, CallError>) {
        self.answers.insert(id, reply);
    }

    /// No request is filed and none waits for an answer.
    #[must_use]
    pub fn calls_idle(&self) -> bool {
        self.requests.is_empty()
    }
}

/// A handle on the shared state, cloned into each flow.
#[derive(Debug, Clone, Default)]
pub struct Io(Arc<Mutex<Shared>>);

impl Io {
    pub fn with<R>(&self, f: impl FnOnce(&mut Shared) -> R) -> R {
        let mut guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut guard)
    }

    #[must_use]
    pub fn now_ms(&self) -> u64 {
        self.with(|s| s.now_ms)
    }

    /// Ask the host for `call` on leg `leg` and wait for its answer.
    pub async fn call_on(&self, leg: &str, call: VenueCall) -> Result<Answer, CallError> {
        let id = self.with(|s| {
            s.next_id = s.next_id.saturating_add(1);
            let id = s.next_id;
            s.requests.push((id, leg.to_owned(), call));
            id
        });
        Answered { io: self.clone(), id }.await
    }
}

struct Answered {
    io: Io,
    id: u64,
}

impl Future for Answered {
    type Output = Result<Answer, CallError>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.io.with(|s| s.answers.remove(&self.id)) {
            Some(reply) => Poll::Ready(reply),
            None => Poll::Pending,
        }
    }
}

/// Run `future` as far as it goes without an answer.
pub fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}
