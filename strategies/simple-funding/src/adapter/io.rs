//! What the strategy's flows run on: the host's venue requests as awaitable calls, and a poller
//! that drives them with no runtime. A call the flow makes is an [`Intent::Request`] the host
//! carries out as its own work; the flow resumes when the [`Event::Answer`] comes back. A write
//! to the bot's state ([`Persist`]) and an alert ([`Alert`]) leave the same way, in order. The
//! strategy reads no clock (the bot's time is [`View::now_ms`]), no broker and no store.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Poll;
use std::time::Duration;

use bb_strategy_sdk::{
    AccountEquity, Alert, Answer, FundingSample, Intent, MarketFacts, NewOrder, OrderBook,
    OrderResult, Persist, Position, ReadQuery, VenueCall,
};
use rust_decimal::Decimal;

/// A point on the bot's clock, in milliseconds: what the strategy measures ages and backoffs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mono(pub u64);

impl Mono {
    /// How long `self` is after `earlier`; `None` when `earlier` is the later one.
    #[must_use]
    pub fn checked_duration_since(self, earlier: Self) -> Option<Duration> {
        self.0.checked_sub(earlier.0).map(Duration::from_millis)
    }

    #[must_use]
    pub fn checked_add(self, by: Duration) -> Option<Self> {
        let ms = u64::try_from(by.as_millis()).ok()?;
        self.0.checked_add(ms).map(Self)
    }
}

/// What the host showed of the strategy's one leg, and what the flows have asked of it.
#[derive(Default)]
pub(crate) struct Calls {
    next_id: u64,
    /// Everything the flows asked of the host, in order, not yet handed over.
    pub(crate) out: VecDeque<Intent>,
    /// Requests handed to the host: not yet answered (`None`), or answered and not yet taken.
    answers: HashMap<u64, Option<Result<Answer, FlowError>>>,
    /// Acked writes handed over, likewise.
    persisted: HashMap<u64, Option<Result<(), String>>>,
    pub(crate) now_ms: u64,
    pub(crate) exchange: String,
    pub(crate) symbols: Vec<String>,
    pub(crate) market: BTreeMap<String, MarketFacts>,
    /// The strategy asked to stop the bot.
    pub(crate) shutdown: Option<String>,
}

#[derive(Clone, Default)]
pub struct Shared(Arc<Mutex<Calls>>);

impl Shared {
    pub(crate) fn lock(&self) -> MutexGuard<'_, Calls> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether anything is out to the host and not answered.
    pub(crate) fn waits_on_host(&self) -> bool {
        let calls = self.lock();
        !calls.answers.is_empty() || !calls.persisted.is_empty()
    }

    pub(crate) fn answer(&self, id: u64, answer: Option<Result<Answer, FlowError>>) {
        let mut calls = self.lock();
        if let Some(slot) = calls.answers.get_mut(&id) {
            *slot = Some(answer.unwrap_or_else(|| Err(FlowError::strategy("an empty answer"))));
        }
    }

    pub(crate) fn persisted(&self, id: u64, result: Result<(), String>) {
        if let Some(slot) = self.lock().persisted.get_mut(&id) {
            *slot = Some(result);
        }
    }

    /// An alert for the owner (the notifier throttles repeats).
    pub(crate) fn raise(&self, key: &str, text: String) {
        let alert = Alert::Raise { key: key.to_string(), text };
        self.lock().out.push_back(Intent::Notify(alert));
    }

    pub(crate) fn resolve(&self, key: &str) {
        self.lock().out.push_back(Intent::Notify(Alert::Resolve { key: key.to_string() }));
    }

    /// A write nothing waits for.
    pub(crate) fn persist(&self, key: &str, payload: serde_json::Value) {
        self.lock().out.push_back(Intent::Persist(Persist::new(key, payload)));
    }

    /// A write the host applies before anything else of its round; `Err` is the store's text.
    pub(crate) async fn persist_acked(
        &self,
        key: &str,
        payload: serde_json::Value,
    ) -> Result<(), String> {
        let id = {
            let mut calls = self.lock();
            calls.next_id = calls.next_id.wrapping_add(1);
            let id = calls.next_id;
            calls.out.push_back(Intent::Persist(Persist::acked(key, payload, id)));
            calls.persisted.insert(id, None);
            id
        };
        let shared = self.clone();
        std::future::poll_fn(move |_| {
            let mut calls = shared.lock();
            match calls.persisted.get_mut(&id).and_then(Option::take) {
                Some(result) => {
                    calls.persisted.remove(&id);
                    Poll::Ready(result)
                }
                None => Poll::Pending,
            }
        })
        .await
    }

    /// Ask the host for `call` and wait for its answer. Lazy: the host is asked on the first poll,
    /// so a call made but never awaited (a dropped future) sends nothing.
    async fn call(&self, call: VenueCall) -> Result<Answer, FlowError> {
        let shared = self.clone();
        let mut ask = Some(call);
        let mut asked: Option<u64> = None;
        std::future::poll_fn(move |_| {
            let mut calls = shared.lock();
            let id = match (asked, ask.take()) {
                (Some(id), _) => id,
                (None, Some(call)) => {
                    calls.next_id = calls.next_id.wrapping_add(1);
                    let id = calls.next_id;
                    let exchange = calls.exchange.clone();
                    calls.out.push_back(Intent::Request { id, exchange, call });
                    calls.answers.insert(id, None);
                    asked = Some(id);
                    id
                }
                (None, None) => return Poll::Pending,
            };
            match calls.answers.get_mut(&id).and_then(Option::take) {
                Some(answer) => {
                    calls.answers.remove(&id);
                    Poll::Ready(answer)
                }
                None => Poll::Pending,
            }
        })
        .await
    }
}

fn unexpected(answer: &Answer) -> FlowError {
    FlowError::strategy(format!("the host answered a call with {answer:?}"))
}

/// The venue as the flows see it: every read, write and wait a request to the host, and the
/// venue's facts as the host's last view showed them.
pub struct Venue {
    shared: Shared,
}

impl Venue {
    pub(crate) fn new(shared: Shared) -> Self {
        Self { shared }
    }

    async fn read(&self, query: ReadQuery) -> Result<Answer, FlowError> {
        self.shared.call(VenueCall::Read(query)).await
    }

    /// The bot's time.
    #[must_use]
    pub fn now(&self) -> Mono {
        Mono(self.shared.lock().now_ms)
    }

    /// The bot's time as unix milliseconds.
    #[must_use]
    pub fn unix_ms(&self) -> u64 {
        self.shared.lock().now_ms
    }

    /// Wait until `deadline` on the bot's clock (the host serves the wait, also while winding
    /// down).
    pub async fn sleep_until(&self, deadline: Mono) {
        let _ = self.shared.call(VenueCall::Wait { until_ms: deadline.0 }).await;
    }

    fn facts(&self, symbol: &str) -> MarketFacts {
        self.shared.lock().market.get(symbol).cloned().unwrap_or_default()
    }

    #[must_use]
    pub fn step_size(&self, symbol: &str) -> Option<Decimal> {
        self.facts(symbol).step_size
    }

    #[must_use]
    pub fn taker_fee_bps(&self, symbol: &str) -> Option<Decimal> {
        self.facts(symbol).taker_fee_bps
    }

    /// The symbols the venue lists.
    #[must_use]
    pub fn available_symbols(&self) -> Vec<String> {
        self.shared.lock().symbols.clone()
    }

    /// Ask the host to stop the bot.
    pub fn request_shutdown(&self) {
        self.shared.lock().shutdown.get_or_insert_with(|| "the strategy asked to stop".into());
    }

    /// # Errors
    /// The host's own error for the call.
    pub async fn get_positions(&self) -> Result<Vec<Position>, FlowError> {
        match self.read(ReadQuery::Positions).await? {
            Answer::Positions(p) => Ok(p),
            other => Err(unexpected(&other)),
        }
    }

    /// # Errors
    /// The host's own error for the call.
    pub async fn get_orderbook(&self, symbol: &str, depth: usize) -> Result<OrderBook, FlowError> {
        match self.read(ReadQuery::Book { symbol: symbol.to_string(), depth }).await? {
            Answer::Book(book) => Ok(book),
            other => Err(unexpected(&other)),
        }
    }

    /// # Errors
    /// The host's own error for the call.
    pub async fn get_account_equity(&self) -> Result<Option<AccountEquity>, FlowError> {
        match self.read(ReadQuery::AccountEquity).await? {
            Answer::AccountEquity(e) => Ok(e),
            other => Err(unexpected(&other)),
        }
    }

    /// # Errors
    /// The host's own error for the call.
    pub async fn margin_ratio(&self) -> Result<Option<Decimal>, FlowError> {
        match self.read(ReadQuery::MarginRatio).await? {
            Answer::MarginRatio(r) => Ok(r),
            other => Err(unexpected(&other)),
        }
    }

    /// # Errors
    /// The host's own error for the call.
    pub async fn funding_history(
        &self,
        symbol: &str,
        start_ms: i64,
    ) -> Result<Vec<FundingSample>, FlowError> {
        let query = ReadQuery::FundingHistory { symbol: symbol.to_string(), start_ms };
        match self.read(query).await? {
            Answer::FundingHistory(h) => Ok(h),
            other => Err(unexpected(&other)),
        }
    }

    /// # Errors
    /// The host's own error for the call.
    pub async fn open_interest(&self, symbol: &str) -> Result<Option<Decimal>, FlowError> {
        match self.read(ReadQuery::OpenInterest { symbol: symbol.to_string() }).await? {
            Answer::OpenInterest(oi) => Ok(oi),
            other => Err(unexpected(&other)),
        }
    }

    /// # Errors
    /// The host's own error for the call.
    pub async fn refresh_metadata(&self) -> Result<(), FlowError> {
        self.read(ReadQuery::RefreshMetadata).await.map(|_| ())
    }

    /// # Errors
    /// The host's own error for the call; one that is not `Strategy` or `Config` may have reached
    /// the venue.
    pub async fn place_orders(&self, orders: &[NewOrder]) -> Result<Vec<OrderResult>, FlowError> {
        match self.shared.call(VenueCall::Place(orders.to_vec())).await? {
            Answer::Placed(r) => Ok(r),
            other => Err(unexpected(&other)),
        }
    }
}

/// The host's error for a call as the flows classify it: a refusal before the venue is a local
/// (`Strategy`) error, anything else may have reached it (an exchange error, retryable as told).
pub(crate) fn to_bot(e: bb_strategy_sdk::CallError) -> FlowError {
    if e.never_sent {
        FlowError::strategy(e.message)
    } else {
        FlowError::Exchange { message: e.message, retryable: e.retryable }
    }
}

/// Why a flow failed, worded as the host's errors always were (the transcripts pin the words).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowError {
    /// A refusal before the venue (or a local fault): the call never left.
    Strategy(String),
    Config(String),
    /// Anything else: the call may have reached the venue; the host words it already.
    Exchange {
        message: String,
        retryable: bool,
    },
}

impl FlowError {
    pub(crate) fn strategy(message: impl Into<String>) -> Self {
        Self::Strategy(message.into())
    }

    pub(crate) fn config(message: impl Into<String>) -> Self {
        Self::Config(message.into())
    }

    /// Whether the bot cannot go on: a local fault, or an exchange error that will not pass.
    pub(crate) fn is_fatal(&self) -> bool {
        match self {
            Self::Strategy(_) | Self::Config(_) => true,
            Self::Exchange { retryable, .. } => !retryable,
        }
    }
}

impl std::fmt::Display for FlowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Strategy(m) => write!(f, "Strategy error: {m}"),
            Self::Config(m) => write!(f, "Configuration error: {m}"),
            Self::Exchange { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for FlowError {}
