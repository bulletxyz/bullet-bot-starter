//! The port a strategy implements, as bb-strategy's host runs it: data in, intents out. A
//! strategy never touches a broker, a clock or a store. Same shape as
//! `bb_strategy::StrategyAdapter` and `StrategyReport`, so a strategy moves between the two by
//! changing its imports.

use std::fmt;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::calls::{Answer, CallError, VenueCall};
use crate::history::{HistoryNeed, Warmup};
use crate::legs::LegView;
use crate::types::{
    AmendOrder, CancelOrder, NewOrder, Order, OrderBook, OrderResult, Position, Trade,
};

/// A strategy: pure decisions over the host's data.
pub trait StrategyAdapter: Send + 'static {
    /// The strategy's own state, published in the host's snapshot.
    type Report: StrategyReport;

    /// Once, before any event: the venue's facts and the market now. Errors stop the build.
    ///
    /// # Errors
    /// A [`StrategyError`] when the strategy cannot run (e.g. a config it cannot use).
    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError>;

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent>;

    /// The bot is winding down (any reason): what to send before it stops.
    fn stop(&mut self, view: &View<'_>) -> Vec<Intent>;

    /// The strategy's own extras (levels, signals). Position, entry, P&L, fills and orders are the
    /// host's; a strategy never reports them itself.
    fn report(&self) -> Self::Report;

    /// The past candles this strategy warms up on, asked once after it is built: the host loads
    /// them before [`Self::start`] and hands them to [`Self::warm_up`]. `None` (the default): it
    /// needs no history.
    fn history(&self) -> Option<HistoryNeed> {
        None
    }

    /// Once, before [`Self::start`], for a strategy whose [`Self::history`] asked: the bars, or why
    /// there are none. Without them it warms up from live data, as it would with no history.
    fn warm_up(&mut self, _warmup: Warmup) {}
}

/// How a component builds its strategy: from the settings the host loads it with (JSON text).
pub trait FromSettings: Sized {
    /// # Errors
    /// A [`StrategyError`] naming what in the settings it cannot use; the host refuses the load.
    fn from_settings(settings: &str) -> Result<Self, StrategyError>;
}

/// A strategy's own state, as the fields it shows: the host builds the report (and its schema)
/// from them, so a strategy never sends JSON or a schema of its own. What a generic reader needs
/// is a method here, with a default when only some strategies have it.
pub trait StrategyReport: fmt::Debug + Send + Sync + 'static {
    /// One line on what the strategy is doing now, in its own words.
    fn headline(&self) -> String;

    /// What it shows of itself, in order: at most 128 fields, a name of 1 to 64 characters, a
    /// text of at most 200 and a unit of at most 16 (the host faults a report past them).
    fn fields(&self) -> Vec<ReportField>;

    /// What the paper report needs from a strategy that quotes: `None` for one that doesn't.
    fn paper_inputs(&self) -> Option<PaperInputs> {
        None
    }

    /// Sentences the strategy says about itself in the briefing, each ending in a space, in order.
    fn briefing_lines(&self) -> Vec<String> {
        Vec::new()
    }

    /// The price the strategy values its own orders at without a fresh book. `None`: none.
    fn reference_price(&self) -> Option<Decimal> {
        None
    }
}

/// The paper report's inputs only a quoting strategy has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaperInputs {
    /// The size of one order.
    pub order_size: Decimal,
    /// Seconds this run spent paper trading with a quote up and no stop in force.
    pub quoting_secs: u64,
    /// The maker fee the config names, in basis points (0 when none is configured).
    pub maker_fee_bps: Decimal,
}

/// One field of a report: a name, its value and its unit, if it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportField {
    pub name: String,
    pub value: FieldValue,
    pub unit: Option<String>,
}

/// A report field's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    Number(Decimal),
    Text(String),
    Flag(bool),
}

impl ReportField {
    /// A number named `name`.
    #[must_use]
    pub fn number(name: impl Into<String>, value: Decimal) -> Self {
        Self { name: name.into(), value: FieldValue::Number(value), unit: None }
    }

    /// A text named `name`.
    #[must_use]
    pub fn text(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self { name: name.into(), value: FieldValue::Text(value.into()), unit: None }
    }

    /// A flag named `name`.
    #[must_use]
    pub fn flag(name: impl Into<String>, value: bool) -> Self {
        Self { name: name.into(), value: FieldValue::Flag(value), unit: None }
    }

    /// This field in `unit`.
    #[must_use]
    pub fn unit(self, unit: impl Into<String>) -> Self {
        Self { unit: Some(unit.into()), ..self }
    }
}

/// What the host shows a strategy on every call.
#[derive(Debug, Clone)]
pub struct View<'a> {
    /// The bot's clock, Unix milliseconds; a strategy never reads the wall clock.
    pub now_ms: u64,
    pub symbol: &'a str,
    pub book: Option<&'a OrderBook>,
    /// The host's inventory from fills, seeded from the venue position at start.
    pub position: Decimal,
    pub avg_entry: Option<Decimal>,
    pub realized_pnl: Decimal,
    /// The strategy's resting orders from the venue read the host just made: `Some` only for the
    /// view `start` gets and a tick's first round when its read succeeded. `None` otherwise: the
    /// list is unknown or stale, so a strategy must not treat a level as gone.
    pub open_orders: Option<&'a [Order]>,
    /// The venue's positions from the same read as `open_orders`, with the same freshness: `Some`
    /// only for the view `start` gets and a tick's first round when its read succeeded. A strategy
    /// sizes its closes and reconciles its inventory on this ([`crate::signed_position`]).
    pub positions: Option<&'a [Position]>,
    /// The venue stream reconnected since the strategy last saw a tick: events may have been
    /// missed, so a strategy that tracks orders or inventory reconciles them now. True on one
    /// tick's view, then false.
    pub reconnected: bool,
    /// The venue refuses orders (why), e.g. a stale oracle on its side: a strategy stops quoting
    /// until this is `None` again.
    pub venue_halt: Option<&'a str>,
    pub market: MarketFacts,
    /// Every other leg the host was given (the strategy's [`LegNeed`](crate::LegNeed)s). Empty
    /// for a strategy on one symbol.
    pub legs: &'a [LegView],
    /// The host's own order path refuses new orders (why): its stop, not the venue's. Refusals it
    /// causes are not the venue's rejections.
    pub path_stop: Option<&'a str>,
    /// When the venue read behind `open_orders` started (the bot's clock, ms); `None` without a
    /// fresh read. A strategy that decides on what a read omits compares this with when it sent
    /// the order.
    pub read_started_ms: Option<u64>,
}

/// The venue's facts about the market, when it reports them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketFacts {
    pub tick_size: Option<Decimal>,
    pub step_size: Option<Decimal>,
    pub maker_fee_bps: Option<Decimal>,
    pub taker_fee_bps: Option<Decimal>,
    /// The account's fee tier, when the venue has tiers.
    pub fee_tier: Option<u64>,
    /// The venue's name as a person reads it, for text the strategy words for the owner.
    pub venue_name: Option<String>,
}

/// Something that happened, for the strategy to react to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    /// The book changed; the book itself is in the [`View`].
    Book,
    /// The venue's mark price for the host's symbol.
    Mark(Decimal),
    Fill(Trade),
    Order(Order),
    Tick,
    /// What the venue (or the gate) answered for one intent, matched by client id.
    Outcome {
        intent: Intent,
        result: OrderResult,
    },
    /// `event` (a book, a mark, a fill, an order update) for `symbol` on the leg `leg`; the
    /// leg's books, marks and positions are in [`View::legs`].
    Leg {
        leg: String,
        symbol: String,
        event: Box<Event>,
    },
    /// The answer to the [`Intent::Request`] with this `id`: exactly one per request.
    Answer {
        id: u64,
        reply: Result<Answer, CallError>,
    },
    /// The call that carried out the place or amend `intent` errored after it may have reached
    /// the venue: the order may be resting or filled. Hold its slot (never place it again as if
    /// refused) until a venue read that started after it lists or omits it.
    Unknown {
        intent: Intent,
        error: String,
    },
}

/// What a strategy asks the host to do. Every order passes the host's gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Intent {
    Place(NewOrder),
    Cancel(CancelOrder),
    CancelAll,
    Amend(AmendOrder),
    /// Stop the bot, with the reason (e.g. a config that can't run).
    Shutdown(String),
    /// `intent` on the leg `leg` (one the strategy declared, [`TRADE_LEG`](crate::TRADE_LEG) for
    /// the one the host was built over), through that leg's gate. A place, a cancel, a cancel
    /// all, an amend or a flatten: a leg inside a leg, a request or a shutdown on a leg is not
    /// sent.
    Leg {
        leg: String,
        intent: Box<Intent>,
    },
    /// Close the venue's position in each of `symbols` (reduce-only IOCs at the touch, capped
    /// from mid), on the leg it is sent to; its outcome succeeds only when each reads flat. On the
    /// owner's main account (not a sub-account of the instance's own) the host refuses it on the
    /// symbol it trades: part of that position may be the owner's.
    Flatten {
        symbols: Vec<String>,
    },
    /// A venue call on leg `leg`, carried out by the host between events and answered once by an
    /// [`Event::Answer`] with the same `id`. The strategy picks the ids; one in flight is never
    /// reused.
    Request {
        id: u64,
        leg: String,
        call: VenueCall,
    },
}

/// A strategy could not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyError(pub String);

impl fmt::Display for StrategyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StrategyError {}
