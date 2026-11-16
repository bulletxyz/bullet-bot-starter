//! The port a strategy implements: data in, intents out. A strategy never touches a broker, a
//! clock or a store: the host feeds it [`Event`]s with a [`View`] of the market and the bot's own
//! state, and carries out the [`Intent`]s it returns through its order path. This is the one trait
//! for the bot's built-in strategies (run natively) and for WASM plugins (through the `bb:strategy`
//! world). The owned data types derive serde so a WASM host can pass them across the boundary.

use std::fmt;
use std::time::Duration;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::history::candles::{HistoryNeed, Warmup};
use crate::host::calls::{Answer, CallError, VenueCall};
use crate::host::capability::{Alert, CloseReport, LimitsUpdate, Persist, ReferencePrice};
use crate::host::control::{ControlCmd, ControlReply};
use crate::host::legs::LegView;
use crate::stops::StopReason;
use crate::strategy::twin::{TwinAdapter, TwinSpec};
use crate::targets::Target;
use crate::types::{
    AmendOrder, CancelOrder, NewOrder, Order, OrderBook, OrderResult, Position, Protection, Trade,
};

pub use crate::strategy::report::{FieldValue, PaperInputs, ReportField, StrategyReport};

/// Whether a run is live or on paper. Only a paper run may start leniently (before its key is
/// delegated), and only a live one cancels the venue's orders when it winds down: a paper bot must
/// never touch the owner's real orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartMode {
    Live,
    Paper,
}

/// The book a strategy that trades on paper by itself keeps (its core fills its own quotes): what
/// the snapshot shows in place of the host's inventory, which paper fills never reach.
#[derive(Debug, Clone, Default)]
pub struct OwnBook {
    pub position: Decimal,
    pub avg_entry: Option<Decimal>,
    pub realized_pnl: Decimal,
    pub fills: u64,
    /// Of `fills`, the paper (dry run) ones.
    pub paper_fills: u64,
    pub open_orders: Vec<Order>,
}

/// What a strategy knows about itself that the host does not, for the snapshot: its own stop and
/// halt, its day P&L, the venue's fee tier and the live position a paper run found, and the quotes
/// it would show as its open orders. Each `None` leaves what the host knows.
#[derive(Debug, Clone, Default)]
pub struct SnapshotExtras {
    /// The strategy's own stop (the host's order path may have one too; this one wins).
    pub stop: Option<StopReason>,
    /// Why the orders are pulled for now with no stop in force.
    pub halt: Option<String>,
    pub day_pnl_net: Option<Decimal>,
    /// The realized P&L since start as the strategy counts it.
    pub realized_pnl: Option<Decimal>,
    /// Today's fills and, of those, the paper fills, as the strategy counts them (restored from
    /// its history at start): shown in place of the host's count of what it saw.
    pub fills_today: Option<u64>,
    pub paper_fills_today: Option<u64>,
    pub fee_tier: Option<u64>,
    pub live_position_in_paper: Option<Decimal>,
    /// Shown as the open orders in place of the venue's (a market maker's quotes).
    pub quotes: Option<Vec<Order>>,
}

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

    /// The host cancelled every order on the symbol (the wind down's cancel-all succeeded): what
    /// the strategy showed as resting is gone. Not called when the cancel failed or was not sent,
    /// so a failed cancel leaves the quotes showing.
    fn orders_cancelled(&mut self) {}

    /// Whether this tick needs a fresh read of the venue (`View::open_orders`, `View::positions`).
    /// The host reads at a tick only when this says so, the order path has a fill or cancel
    /// waiting for a read that follows it, or the venue stream reconnected (`reconnected`); a
    /// tick without a read makes no REST call and its view carries no venue lists. The default is
    /// every tick, for a strategy that reconciles against the venue each time.
    fn wants_venue(&self, _now_ms: u64, _reconnected: bool) -> bool {
        true
    }

    /// How often the strategy wants [`Event::Tick`]: the host delivers one when at least this long
    /// has passed since the last one it delivered (on the bot clock). `None` (the default) is every
    /// tick the bot makes. A strategy declares its cadence here instead of counting ticks itself.
    fn tick_interval(&self) -> Option<Duration> {
        None
    }

    /// A twin of this strategy with `spec.changes` staged on top of its settings, for the host to
    /// run beside it (a second, dry-run instance that never trades). `Ok(None)` (the default):
    /// this strategy cannot be staged.
    ///
    /// # Errors
    /// A staged value is out of bounds or unknown (the text says which).
    fn twin(&self, _spec: &TwinSpec) -> Result<Option<TwinAdapter>, String> {
        Ok(None)
    }

    /// Live (the default) or paper: see [`StartMode`]. The host starts a paper run leniently when
    /// asked to, seeds no inventory from the venue for it, and never cancels the venue's orders
    /// when it winds down.
    fn start_mode(&self) -> StartMode {
        StartMode::Live
    }

    /// The book of a strategy that trades on paper by itself, for the snapshot (see [`OwnBook`]).
    fn own_book(&self) -> Option<OwnBook> {
        None
    }

    /// What the snapshot shows beyond what the host knows (see [`SnapshotExtras`]).
    fn snapshot_extras(&self) -> SnapshotExtras {
        SnapshotExtras::default()
    }

    /// Keys the strategy puts at the top level of its actor's `/status` (frozen for a strategy that
    /// has always shown them), beside the report and the host's view.
    fn status_keys(&self) -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::new()
    }

    /// Whether the strategy has faulted (a plugin that trapped, ran out of fuel or broke its
    /// contract): what it returns from now on is the host's own cancel-all of what it left
    /// resting, which the host carries out whatever the cancel cap says. Never true for a
    /// strategy that can't fault.
    fn faulted(&self) -> bool {
        false
    }

    /// The past candles this strategy warms up on, asked once after it is built: the host loads
    /// them before [`Self::start`] and hands them to [`Self::warm_up`]. `None` (the default): it
    /// needs no history.
    fn history(&self) -> Option<HistoryNeed> {
        None
    }

    /// Once, before [`Self::start`], for a strategy whose [`Self::history`] asked: the bars, or why
    /// there are none. Without them it warms up from live data, as it would with no history, and
    /// says so in its report.
    fn warm_up(&mut self, _warmup: Warmup) {}

    /// Every series of past candles this strategy warms up on, for one that needs more than one
    /// interval (native only: the WIT world's `history` keeps one). The host loads each before
    /// [`Self::start`] and hands it to [`Self::warm_up_for`]. The default is [`Self::history`]'s
    /// one need.
    fn histories(&self) -> Vec<HistoryNeed> {
        self.history().into_iter().collect()
    }

    /// Once for each of [`Self::histories`], before [`Self::start`]: the bars loaded for `need`, or
    /// why there are none. The default hands them to [`Self::warm_up`].
    fn warm_up_for(&mut self, _need: HistoryNeed, warmup: Warmup) {
        self.warm_up(warmup);
    }

    /// The stop (and take-profit) this strategy wants the venue to hold for the position it has
    /// now, so the position stays guarded while the bot is down. `None` (the default): none. The
    /// host keeps the venue's stop at it, live, on a venue that holds stops; the strategy's own
    /// stop keeps running beside it.
    fn protective_stop(&self, _view: &View<'_>) -> Option<Protection> {
        None
    }

    /// Once, before [`Self::start`], of a live start that takes over the position a kept shutdown
    /// left (`instance.on_shutdown = keep`): the host seeds the inventory from the venue and
    /// cancels every order on the market first, as at any live start; a strategy that refuses a
    /// held position at start (the grid) goes on from this one.
    fn reclaim(&mut self) {}

    /// Before [`Self::start`]: the state the host kept of this instance, read through `state`. The
    /// default reads nothing. An error stops the start.
    ///
    /// # Errors
    /// The saved state cannot be used; the start is refused.
    fn restore(&mut self, _state: &dyn StateRead) -> Result<(), StrategyError> {
        Ok(())
    }

    /// Whether a kept shutdown may leave the position and orders as they are now. `false` while
    /// the strategy's own stop or close is under way: that shutdown closes as any other does.
    fn may_keep(&self) -> bool {
        true
    }
}

/// The state the host kept of an instance, as text documents named by the strategy's own writes
/// ([`Intent::Persist`]): read-only, for [`StrategyAdapter::restore`].
pub trait StateRead {
    /// Whether the host keeps state for this instance at all (a run without a state directory does
    /// not).
    fn keeps_state(&self) -> bool;

    /// The document `name` (`None`: nothing saved under it).
    ///
    /// # Errors
    /// The store could not be read.
    fn doc(&self, name: &str) -> Result<Option<String>, String>;

    /// Every line of the append-only document `name`, oldest first (an unreadable line is skipped).
    ///
    /// # Errors
    /// The store could not be read.
    fn lines(&self, name: &str) -> Result<Vec<String>, String>;
}

/// How a component builds its strategy: from the settings the host loads it with (JSON text).
pub trait FromSettings: Sized {
    /// # Errors
    /// A [`StrategyError`] naming what in the settings it cannot use; the host refuses the load.
    fn from_settings(settings: &str) -> Result<Self, StrategyError>;
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
    /// The strategy's orders resting on the venue, from the venue read the host just made: `Some`
    /// only for the view `start` gets and for a tick's first round when its read succeeded. `None`
    /// for every other view (a fill, an order update, a book, an outcome, a later round, `stop`)
    /// and when a tick's read failed: the list is then unknown or stale, so a strategy must not
    /// treat a level as gone.
    pub open_orders: Option<&'a [Order]>,
    /// The venue's positions from the same read as `open_orders`, with the same freshness: `Some`
    /// only for the view `start` gets and for a tick's first round when its read succeeded, `None`
    /// otherwise. A strategy sizes its closes and reconciles its inventory on this
    /// ([`crate::signed_position`]); it never reads the venue.
    pub positions: Option<&'a [Position]>,
    /// The venue stream reconnected since the strategy last saw a tick: events may have been
    /// missed, so a strategy that tracks orders or inventory reconciles them against `open_orders`
    /// and `positions` now. True on one tick's view, then false.
    pub reconnected: bool,
    /// The venue refuses orders (why), e.g. a stale oracle on its side. A strategy stops quoting,
    /// as it would under a stop of its own, until this is `None` again.
    pub venue_halt: Option<&'a str>,
    pub market: MarketFacts,
    /// The host's other legs (the strategy's [`LegNeed`](crate::LegNeed)s): each one's venue,
    /// declared symbols, books, marks and the positions its last read found (`None`: unread,
    /// never flat). Empty for a strategy on one symbol.
    pub legs: &'a [LegView],
    /// The host's own order path refuses new orders (why): its stop, not the venue's. Refusals it
    /// causes are not rejections by the venue, so a strategy's reject breaker must not count them.
    pub path_stop: Option<&'a str>,
    /// When the venue read behind `open_orders` and `positions` started (the bot clock, ms);
    /// `None` with no fresh read. A strategy that decides on what a read omits compares it with
    /// when the thing it asks about was sent, not with when the read reached it.
    pub read_started_ms: Option<u64>,
    /// The hard limits the host holds the order path to (native only; `None` for a plugin), live:
    /// a paper run leaves it `None`, so a paper cap nobody chose refuses no plan.
    pub limits: Option<LimitsInForce>,
    /// The host's own stop, typed (native only; `None` for a plugin): what `path_stop` says in
    /// words, so a strategy can tell a daily loss stop from a pause.
    pub stop: Option<&'a StopReason>,
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

/// The limits in force, as the snapshot shows them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitsInForce {
    pub max_position: Decimal,
    pub daily_loss_usd: Decimal,
    pub stale_secs: Option<u64>,
    /// The most notional (USD) the path holds open, when there is a cap (left out of the JSON
    /// when there is none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_notional_usd: Option<Decimal>,
}

impl LimitsInForce {
    #[must_use]
    pub fn new(max_position: Decimal, daily_loss_usd: Decimal, stale_secs: Option<u64>) -> Self {
        Self { max_position, daily_loss_usd, stale_secs, max_notional_usd: None }
    }
}

/// Something that happened, for the strategy to react to. Exhaustive on purpose, like [`Intent`]: a
/// new case changes every adapter and the `bb:strategy` WIT world, and the WASM host matches every
/// case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    /// The book changed; the book itself is in the [`View`].
    Book,
    /// The venue's mark price for the host's symbol, on its own venue (a strategy that works on the
    /// mark, not the book).
    Mark(Decimal),
    Fill(Trade),
    Order(Order),
    Tick,
    /// What the venue (or the gate) answered for one intent the strategy returned, matched by
    /// client id: a place, an amend or a cancel.
    Outcome {
        intent: Intent,
        result: OrderResult,
    },
    /// A price from the reference feed the host was wired to (see [`ReferencePrice`]). Only
    /// delivered; it asks nothing of the strategy but to record it.
    Reference(ReferencePrice),
    /// A command from outside (an MCP server, Telegram, the owner's UI). Every command gets exactly
    /// one [`Intent::Reply`] under `id`, at once or later, from this event or a later one; a
    /// command the strategy cannot carry out is answered with [`ControlReply::Refused`]. `id` is
    /// the host's.
    Control {
        id: u64,
        cmd: ControlCmd,
    },
    /// An acked [`Persist`] was applied (`Ok`) or failed (`Err`, the store's text), by its `ack`
    /// id.
    Persisted {
        id: u64,
        result: Result<(), String>,
    },
    /// The close [`Intent::ClosePosition`] asked for ended; see [`CloseReport`].
    Closed {
        id: u64,
        report: CloseReport,
    },
    /// The call that carried out `intent` failed as a whole (a transport or venue error, not a
    /// refusal of this intent): nothing is known of it, and the error also ends the handler. Unlike
    /// an [`Event::Outcome`] with `success: false`, it says nothing about the intent itself.
    Failed {
        intent: Intent,
        error: String,
    },
    /// The call that carried out the place or amend `intent` errored after it may have reached
    /// the venue (a timeout, a 5xx): the order may be resting or filled. The host counts it against
    /// its limits until a venue read that started after it lists or omits it; a strategy holds
    /// the order's slot instead of re-placing it or acting on it as refused.
    Unknown {
        intent: Intent,
        error: String,
    },
    /// `event` (a book, a mark, a fill, an order update) for `symbol` on the leg `leg`; the leg's
    /// books, marks and positions are in [`View::legs`].
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
    /// The funding executor's market view for this decision tick (signals, costs, marks, open
    /// interest, each venue's legs, equity, the knobs the strategy decides within). Answer it with
    /// [`Intent::Targets`]. Only a host that owns a funding executor delivers it.
    MarketView(crate::targets::MarketView),
}

impl Event {
    /// The `kind` label of the step latency histogram: the market maker core's `Input::kind` for
    /// the same input, so its series keep their names.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Book => "book",
            Self::Mark(_) => "mark",
            Self::Fill(_) => "trade",
            Self::Order(_) => "lifecycle",
            Self::Tick => "tick",
            Self::Reference(_) => "reference",
            Self::Leg { event, .. } => event.kind(),
            Self::Control { .. } => "control",
            Self::Persisted { .. } => "persisted",
            Self::Outcome { .. }
            | Self::Closed { .. }
            | Self::Failed { .. }
            | Self::Unknown { .. }
            | Self::Answer { .. } => "done",
            Self::MarketView(_) => "market_view",
        }
    }
}

/// What a strategy asks the host to do (exhaustive, see [`Event`]). Every order passes the host's gate. A plugin
/// faults on `Reply`, `Persist`, `ClosePosition`, `SetLimits`, `StandDown` or `Targets`; its `CancelOwn` crosses as
/// `CancelAll` (its own orders only) and its `Notify` is dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Intent {
    Place(NewOrder),
    Cancel(CancelOrder),
    /// Cancel every order on the market, the owner's too: a safety stop's pull. On a leg, and in a
    /// normal wind-down's stop round, it takes only the bot's own orders.
    CancelAll,
    /// Cancel only the bot's own orders on the market (those with its marked client id, and those
    /// its path sent): the owner's orders there stay. Live, when the orders can't be listed or its
    /// own can't all be cancelled, every order goes.
    CancelOwn,
    Amend(AmendOrder),
    /// Stop the bot, with the reason (e.g. a config that can't run).
    Shutdown(String),
    /// Hold the instance for safety, with the reason: the supervisor's safety hold, kept through
    /// a restart until the owner resumes it. The strategy closes what it holds itself first, in
    /// the same round or before.
    StandDown(String),
    /// Answer the command [`Event::Control`] delivered under `id`. The host sends the reply to the
    /// caller; an id it does not know (already answered) is ignored.
    Reply {
        id: u64,
        reply: ControlReply,
    },
    /// Write to the bot's state: see [`Persist`]. Carried out before any order of the round; an
    /// acked write holds the rest of the round until it is applied.
    Persist(Persist),
    /// Tell the owner: see [`Alert`]. Lossy by design: a notifier that is full drops it.
    Notify(Alert),
    /// Close the position now with reduce-only IOCs, and answer with [`Event::Closed`] under `id`.
    /// The host cancels every order on the symbol, reads the venue's position, and sends up to
    /// `CLOSE_ATTEMPTS` IOCs at the touch, never more than `CLOSE_MAX_SLIPPAGE_BPS` from mid, each
    /// sized to the position left and through the order path's reduce-only check. Closes only
    /// shrink the position, so this passes a halted path. A strategy that wants to stop quoting
    /// first asks for that (and persists its pause) in an earlier round.
    ClosePosition {
        id: u64,
    },
    /// The strategy tightened its hard limits live: the host tightens the order path's gate and the
    /// snapshot's limits to match. Only a tighter value is applied; a looser one is ignored.
    SetLimits(LimitsUpdate),
    /// `intent` on the leg `leg` (one the strategy declared, [`TRADE_LEG`](crate::TRADE_LEG) for
    /// the one the host was built over): its symbols are checked against that leg's, and it passes
    /// that leg's order path and limits. Only orders, cancels, a cancel all, a flatten and a
    /// shutdown go on a leg; an unknown leg, a leg inside a leg, or anything else is refused.
    Leg {
        leg: String,
        intent: Box<Intent>,
    },
    /// Close the venue's positions in `symbols` (of the leg it is sent on) with the host's bounded
    /// close (`Intent::ClosePosition`'s), each read again after: its outcome succeeds only when
    /// every one reads flat. It only lowers risk. On the trade leg of an account the owner shares
    /// (Bullet's main account) it is refused: part of that position may be the owner's.
    Flatten {
        symbols: Vec<String>,
    },
    /// A venue call on leg `leg` ([`TRADE_LEG`](crate::TRADE_LEG) for the host's own), carried out
    /// by the host as its own work between events (never in this event's rounds, never over its
    /// budget), bounded by a deadline, and answered once by an [`Event::Answer`] with the same
    /// `id`. Writes pass the leg's order path. The strategy picks the ids; one in flight is never
    /// reused.
    Request {
        id: u64,
        leg: String,
        call: VenueCall,
    },
    /// Hedged target positions, one per symbol: what a funding strategy wants held across its two
    /// venues. The host hands them to the bot's executor, which clamps each to its caps and turns
    /// it into hedged orders; a strategy returning them places no order itself. A symbol left out
    /// is read as target 0.
    Targets(Vec<Target>),
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

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod tests;
