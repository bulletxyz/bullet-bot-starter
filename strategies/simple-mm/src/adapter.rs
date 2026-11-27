//! [`MarketMaker`]: the market maker as a [`StrategyAdapter`]: data in, intents out. The host owns
//! the inventory, the order path and its gate, paper, the guards (daily loss, stale price, fast
//! market, reject breaker, the venue's stale oracle, the reconcile of stray orders, the position
//! resync), the pause, the limits and the state; the market maker decides its two quotes.
//!
//! - `Book` and `Tick`: a refresh once `refresh_secs` has passed since the last (at once after a
//!   pull). Each side: the wanted quote through the cap (`cap.rs`) and, post-only, the cross
//!   check; then nothing, a place, a cancel or a reprice. A reprice is one `Amend` when the
//!   replacement passes the cap with the old quote counted as filled; else the old quote is
//!   cancelled first and the replacement placed on the cancel's answer, re-capped at the position
//!   then (`adapter/quoting.rs`).
//! - `Outcome`, `Failed`, `Unknown`, `Order`, a tick's venue read: what became of each quote
//!   (`adapter/sync.rs`). A quote that may still rest but is no longer a slot (a refused amend, a
//!   failed cancel of one let go) is a stray: it is cancelled first and nothing is placed until it
//!   is gone.
//! - A stop (the host's `path_stop`, `stop` or `venue_halt`, a pause, a close) or a book with no
//!   two sides: no quote, and what rests is pulled (`CancelOwn`, asked again every
//!   [`PULL_RETRY_MS`] while it fails).
//! - `Control` and `Closed`: `adapter/control.rs`.

use bb_strategy_sdk::{
    BULLET, CancelOrder, Event, ExchangeIntent, ExchangeView, Intent, MarketEvent, Order,
    OrderBook, OrderStatus, Side, SnapshotExtras, StrategyAdapter, StrategyError, View,
};
use bb_strategy_support::ClientIdIssuer;
use rust_decimal::Decimal;

use crate::cap::Caps;
use crate::params::{ExtraParam, MmParams};
use crate::pricing::{FairReport, MarketView, Pricing};
use crate::report::MarketMakerReport;
use crate::settings::{MarketMakerSettings, PARAMS};

mod control;
mod quoting;
mod sync;

/// How long after a quote's answer (or after it was let go) a venue read must have started before
/// what it omits is believed: the venue's list can lag.
const SETTLE_MS: u64 = 2_000;
/// How often the venue's open orders are read to drop quotes that are gone.
const READ_EVERY_MS: u64 = 30_000;
/// The least time between two pulls while one fails.
pub const PULL_RETRY_MS: u64 = 10_000;
/// The most one gap between events adds to the time quoting: a stalled bot's quotes were not
/// watched.
const QUOTING_GAP_MS: u64 = 60_000;

/// One side's quote as the market maker believes it rests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slot {
    pub side: Side,
    pub price: Decimal,
    pub size: Decimal,
    pub client_id: String,
    pub order_id: Option<String>,
    /// When its place was answered (bot clock, ms).
    pub acked_ms: u64,
    /// Its place's outcome is unknown: it may rest or not, and a read settles it.
    pub unknown: bool,
}

impl Slot {
    pub fn cancel(&self, symbol: &str) -> CancelOrder {
        CancelOrder {
            symbol: symbol.to_string(),
            order_id: self.order_id.clone().unwrap_or_default(),
            client_id: Some(self.client_id.clone()),
        }
    }
}

/// An order that may still rest though no slot holds it, since `since_ms`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stray {
    pub cancel: CancelOrder,
    pub since_ms: u64,
}

/// What a refresh asked for a side, held until its answer.
#[derive(Debug, Clone)]
pub(crate) enum InFlight {
    Place(Slot),
    /// `old` is replaced by `new` in one message.
    Amend {
        old: Slot,
        new: Slot,
    },
    /// `slot` is cancelled; `then` is placed on the cancel's answer (a two-step reprice).
    Cancel {
        slot: Slot,
        then: Option<Slot>,
    },
    /// A stray's cancel.
    Stray(Stray),
}

/// The pull of every quote (`CancelOwn`) while stopped.
#[derive(Debug, Default)]
pub(crate) struct Pull {
    pub asked: bool,
    pub failed_ms: Option<u64>,
    /// The owner's pause waits for the pull to answer.
    pub pause_reply: Option<u64>,
}

pub struct MarketMaker {
    pub(crate) settings: MarketMakerSettings,
    pub(crate) params: MmParams,
    pub(crate) pricing: Pricing,
    pub(crate) client_ids: ClientIdIssuer,
    /// Bid, ask.
    pub(crate) slots: [Option<Slot>; 2],
    pub(crate) strays: Vec<Stray>,
    pub(crate) in_flight: Vec<InFlight>,
    pub(crate) last_refresh_ms: Option<u64>,
    /// The next refresh is due at once (after a pull, a cancel the host made, a resume).
    pub(crate) forced: bool,
    pub(crate) pull: Pull,
    /// The owner's pause or a close: no quote until a resume.
    pub(crate) paused: bool,
    /// The close under way: its command's id.
    pub(crate) closing: Option<u64>,
    pub(crate) last_read_ms: Option<u64>,
    /// The venue's tick and step, as the last view showed them.
    pub(crate) tick: Option<Decimal>,
    pub(crate) step: Option<Decimal>,
    /// What the last event saw: a stop in force, the fair value, its time.
    pub(crate) halted: bool,
    pub(crate) fair: Option<FairReport>,
    pub(crate) last_event_ms: Option<u64>,
    pub(crate) quoting_ms: u64,
}

pub(crate) fn side_index(side: Side) -> usize {
    match side {
        Side::Buy => 0,
        Side::Sell => 1,
    }
}

impl MarketMaker {
    #[must_use]
    pub fn new(settings: MarketMakerSettings) -> Self {
        let params = MmParams::from_settings(&settings);
        let pricing = Pricing::new(settings.binance_symbol());
        Self {
            settings,
            params,
            pricing,
            client_ids: ClientIdIssuer::default(),
            slots: [None, None],
            strays: Vec::new(),
            in_flight: Vec::new(),
            last_refresh_ms: None,
            forced: true,
            pull: Pull::default(),
            paused: false,
            closing: None,
            last_read_ms: None,
            tick: None,
            step: None,
            halted: false,
            fair: None,
            last_event_ms: None,
            quoting_ms: 0,
        }
    }

    #[must_use]
    pub fn params(&self) -> &MmParams {
        &self.params
    }

    pub(crate) fn symbol(&self) -> &str {
        &self.settings.symbol
    }

    /// Bullet as the host shows it: the exchange its market is on.
    pub(crate) fn bullet<'v>(view: &'v View<'_>) -> Option<&'v ExchangeView> {
        view.exchange(BULLET)
    }

    /// Its market as the host shows it: `symbol` on Bullet.
    pub(crate) fn shown<'v>(&self, view: &'v View<'_>) -> Option<&'v bb_strategy_sdk::MarketView> {
        view.market(BULLET, self.symbol())
    }

    /// The host's position in its market (zero when the market is not shown).
    pub(crate) fn position(&self, view: &View<'_>) -> Decimal {
        self.shown(view).map_or(Decimal::ZERO, |m| m.position)
    }

    pub(crate) fn book<'v>(&self, view: &'v View<'_>) -> Option<&'v OrderBook> {
        self.shown(view).and_then(bb_strategy_sdk::MarketView::book)
    }

    /// `intent` on Bullet, where its market is.
    pub(crate) fn on_bullet(intent: ExchangeIntent) -> Intent {
        Intent::on(BULLET, intent)
    }

    /// The pull: only its own orders on its market.
    pub(crate) fn cancel_own(&self) -> Intent {
        Self::on_bullet(ExchangeIntent::CancelOwn(Some(self.symbol().to_string())))
    }

    pub(crate) fn caps(&self, view: &View<'_>) -> Caps {
        Caps::new(&self.settings, view.limits.as_ref(), self.step)
    }

    /// Why no quote may go out now, if none may: the host's stop, the venue's, a pause or a close.
    pub(crate) fn stopped(&self, view: &View<'_>) -> bool {
        self.paused
            || self.closing.is_some()
            || view.stop.is_some()
            || Self::bullet(view).is_some_and(|e| e.path_stop.is_some() || e.venue_halt.is_some())
    }

    fn quoting(&self) -> bool {
        !self.halted && self.slots.iter().any(Option::is_some)
    }

    /// What the report shows of this event: the time quoting, the stop, the fair value.
    fn seen(&mut self, view: &View<'_>) {
        if let (true, Some(at)) = (self.quoting(), self.last_event_ms) {
            let gap = view.now_ms.saturating_sub(at).min(QUOTING_GAP_MS);
            self.quoting_ms = self.quoting_ms.saturating_add(gap);
        }
        self.last_event_ms = Some(view.now_ms);
        self.halted = self.stopped(view);
        let caps = self.caps(view);
        let mv = market_view(&self.params, caps, self.tick, view, &self.settings.symbol);
        self.fair = self.pricing.fair(&mv);
    }
}

/// What the pricing sees of `symbol` on Bullet in `view`: a book only when it has two sides.
pub(crate) fn market_view<'a>(
    params: &'a MmParams,
    caps: Caps,
    tick: Option<Decimal>,
    view: &'a View<'_>,
    symbol: &str,
) -> MarketView<'a> {
    let shown = view.market(BULLET, symbol);
    let book = shown.and_then(bb_strategy_sdk::MarketView::book).filter(|b| b.midpoint().is_some());
    MarketView {
        book,
        mid: book.and_then(OrderBook::midpoint),
        now_ms: view.now_ms,
        net_position: shown.map_or(Decimal::ZERO, |m| m.position),
        max_position: caps.max_position,
        params,
        tick,
    }
}

impl StrategyAdapter for MarketMaker {
    type Report = MarketMakerReport;

    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        let Some(shown) = self.shown(view) else {
            return Err(StrategyError(format!(
                "The host shows no {} market on {BULLET}: the market maker quotes only that one.",
                self.symbol()
            )));
        };
        self.client_ids = ClientIdIssuer::own(view.now_ms);
        self.tick = shown.facts.tick_size;
        self.step = shown.facts.step_size;
        // The book the host holds at start is the pricing's first.
        let caps = self.caps(view);
        let mv = market_view(&self.params, caps, self.tick, view, &self.settings.symbol);
        self.pricing.on_book(&mv);
        self.seen(view);
        Ok(Vec::new())
    }

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent> {
        let facts = self.shown(view).map(|m| &m.facts);
        self.tick = facts.and_then(|f| f.tick_size).or(self.tick);
        self.step = facts.and_then(|f| f.step_size).or(self.step);
        let intents = match event {
            Event::Market { exchange, symbol, event }
                if exchange == BULLET && symbol == self.symbol() =>
            {
                match event {
                    MarketEvent::Book => self.on_book(view),
                    MarketEvent::Order(order) => {
                        self.on_order(order);
                        Vec::new()
                    }
                    // The host keeps the inventory, and an order's terminal update ends its slot.
                    MarketEvent::Fill(_) | MarketEvent::Mark(_) => Vec::new(),
                }
            }
            Event::Tick => {
                if let Some(open) = Self::bullet(view).and_then(|e| e.open_orders.as_deref()) {
                    self.on_read(open, view);
                }
                self.refresh(view)
            }
            Event::Reference(price) => {
                let caps = self.caps(view);
                let mv = market_view(&self.params, caps, self.tick, view, &self.settings.symbol);
                self.pricing.on_reference(price, &mv);
                Vec::new()
            }
            Event::Outcome { intent, result } => self.on_outcome(intent, result, view),
            Event::Failed { intent, .. } => self.on_failed(intent, view),
            Event::Unknown { intent, .. } => {
                self.on_unknown(intent, view.now_ms);
                Vec::new()
            }
            Event::Control { id, cmd } => self.on_control(*id, cmd, view),
            Event::Closed { id, report } => self.on_closed(*id, report),
            // Another market is not its own; it asks nothing it waits on.
            Event::Market { .. }
            | Event::Persisted { .. }
            | Event::Answer { .. }
            | Event::MarketView(_) => Vec::new(),
        };
        self.seen(view);
        intents
    }

    /// The host's wind-down cancels the quotes, or keeps them under a kept shutdown.
    fn stop(&mut self, view: &View<'_>) -> Vec<Intent> {
        self.seen(view);
        Vec::new()
    }

    fn report(&self) -> MarketMakerReport {
        MarketMakerReport {
            params: self.params.clone(),
            extra_params: PARAMS.iter().map(ExtraParam::from).collect(),
            maker_fee_bps: self.settings.fees.as_ref().map_or(Decimal::ZERO, |f| f.maker_bps),
            paper_quoting_secs: self.quoting_ms / 1000,
            fair_source: self.fair.map(|f| f.source),
            fair_fallback: self.fair.and_then(|f| f.fallback),
            fair_value: self.fair.map(|f| f.price),
            basis_bps: self.fair.and_then(|f| f.basis_bps),
        }
    }

    /// The quotes as it holds them: the venue's last read lags a place or a cancel by a read.
    fn snapshot_extras(&self) -> SnapshotExtras {
        let symbol = self.symbol();
        let quotes = self.slots.iter().flatten().map(|s| Order {
            id: s.order_id.clone().unwrap_or_default(),
            client_id: Some(s.client_id.clone()),
            symbol: symbol.to_string(),
            side: s.side,
            order_type: self.settings.order_type,
            price: s.price,
            quantity: s.size,
            filled_quantity: Decimal::ZERO,
            status: OrderStatus::Open,
        });
        SnapshotExtras { quotes: Some(quotes.collect()), ..SnapshotExtras::default() }
    }

    fn orders_cancelled(&mut self) {
        self.slots = [None, None];
        self.strays.clear();
        self.forced = true;
    }

    fn wants_venue(&self, now_ms: u64, reconnected: bool) -> bool {
        reconnected
            || !self.strays.is_empty()
            || self.slots.iter().flatten().any(|s| s.unknown)
            || self.last_read_ms.is_none_or(|t| now_ms.saturating_sub(t) >= READ_EVERY_MS)
    }

    /// Not while a stop's pull is failing (its quotes must not rest through the downtime), nor
    /// while a close is under way.
    fn may_keep(&self) -> bool {
        self.closing.is_none() && self.pull.failed_ms.is_none()
    }
}

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "adapter_control_tests.rs"]
mod control_tests;
