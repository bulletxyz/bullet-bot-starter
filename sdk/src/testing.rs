//! Helpers for a strategy's own tests (feature `testing`): a market to build views from, and
//! [`SimHost`], a small simulated host that runs a strategy the way bb-strategy's `StrategyHost`
//! does (start, events, each intent's outcome fed back, at most 3 nested rounds), without a venue.
//! Golden replays through the real host live in the host's repository, not here.

use std::collections::BTreeMap;

use crate::Decimal;
use crate::history::candles::{Candle, CandleInterval, Warmup};
use crate::host::legs::LegView;
use crate::strategy::adapter::{Event, Intent, MarketFacts, StrategyAdapter, StrategyError, View};
use crate::types::{NewOrder, Order, OrderBook, OrderResult, Position, Side, Trade};

/// `s` as a decimal; zero when it does not parse (tests write literals).
#[must_use]
pub fn d(s: &str) -> Decimal {
    s.parse().unwrap_or_default()
}

/// A one-level book: `bid` and `ask`, each with `size`.
#[must_use]
pub fn book(bid: &str, ask: &str, size: &str) -> OrderBook {
    OrderBook {
        bids: BTreeMap::from([(d(bid), d(size))]),
        asks: BTreeMap::from([(d(ask), d(size))]),
        last_update_id: 1,
    }
}

/// Closed `interval` bars with these `closes` (open, high and low at the close), oldest first, the
/// last one closing when the bar holding `now_ms` opened: what the host hands a warm-up.
#[must_use]
pub fn closed_candles(interval: CandleInterval, now_ms: u64, closes: &[&str]) -> Vec<Candle> {
    let open = interval.open_of(now_ms);
    let back = |k: usize| {
        let k = u64::try_from(k).unwrap_or(u64::MAX).saturating_add(1);
        open.saturating_sub(k.saturating_mul(interval.ms()))
    };
    closes
        .iter()
        .rev()
        .enumerate()
        .map(|(k, close)| {
            let p = d(close);
            Candle { open_ms: back(k), open: p, high: p, low: p, close: p, volume: Decimal::ONE }
        })
        .rev()
        .collect()
}

/// What a test shows a strategy, owned; [`Market::view`] borrows it as the host would.
#[derive(Debug, Clone, Default)]
pub struct Market {
    pub now_ms: u64,
    pub symbol: String,
    pub book: Option<OrderBook>,
    pub position: Decimal,
    pub avg_entry: Option<Decimal>,
    pub realized_pnl: Decimal,
    pub open_orders: Option<Vec<Order>>,
    pub positions: Option<Vec<Position>>,
    pub reconnected: bool,
    pub venue_halt: Option<String>,
    pub market: MarketFacts,
    pub legs: Vec<LegView>,
    pub path_stop: Option<String>,
    pub read_started_ms: Option<u64>,
    pub limits: Option<crate::LimitsInForce>,
    pub stop: Option<crate::stops::StopReason>,
}

impl Market {
    /// `symbol` with a book of `bid` / `ask`, flat, no orders listed, at `now_ms`.
    #[must_use]
    pub fn new(symbol: &str, bid: &str, ask: &str, now_ms: u64) -> Self {
        Self {
            now_ms,
            symbol: symbol.into(),
            book: Some(book(bid, ask, "10")),
            open_orders: Some(Vec::new()),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn view(&self) -> View<'_> {
        View {
            now_ms: self.now_ms,
            symbol: &self.symbol,
            book: self.book.as_ref(),
            position: self.position,
            avg_entry: self.avg_entry,
            realized_pnl: self.realized_pnl,
            open_orders: self.open_orders.as_deref(),
            positions: self.positions.as_deref(),
            reconnected: self.reconnected,
            venue_halt: self.venue_halt.as_deref(),
            market: self.market.clone(),
            legs: &self.legs,
            path_stop: self.path_stop.as_deref(),
            read_started_ms: self.read_started_ms,
            limits: self.limits.clone(),
            stop: self.stop.as_ref(),
        }
    }
}

/// The places among `intents`.
#[must_use]
pub fn placed(intents: &[Intent]) -> Vec<&NewOrder> {
    intents.iter().filter_map(|i| if let Intent::Place(o) = i { Some(o) } else { None }).collect()
}

/// The venue's answer to `intent`: taken (`Ok`, with an order id) or refused (`Err`, why).
#[must_use]
pub fn outcome(intent: &Intent, answer: Result<&str, &str>) -> Event {
    let client_id = client_id(intent);
    let result = match answer {
        Ok(order_id) => {
            OrderResult { order_id: Some(order_id.into()), client_id, success: true, error: None }
        }
        Err(why) => {
            OrderResult { order_id: None, client_id, success: false, error: Some(why.into()) }
        }
    };
    Event::Outcome { intent: intent.clone(), result }
}

fn client_id(intent: &Intent) -> Option<String> {
    match intent {
        Intent::Place(o) => o.client_id.clone(),
        Intent::Amend(a) => a.new_order.client_id.clone(),
        Intent::Cancel(c) => c.client_id.clone(),
        Intent::Leg { intent, .. } => client_id(intent),
        Intent::CancelAll
        | Intent::CancelOwn
        | Intent::Shutdown(_)
        | Intent::Flatten { .. }
        | Intent::Request { .. }
        | Intent::Reply { .. }
        | Intent::Persist(_)
        | Intent::Notify(_)
        | Intent::ClosePosition { .. }
        | Intent::SetLimits(_)
        | Intent::StandDown(_)
        | Intent::Targets(_) => None,
    }
}

/// A full fill of `order`.
#[must_use]
pub fn fill_of(order: &NewOrder) -> Event {
    Event::Fill(Trade {
        exchange: "sim".into(),
        symbol: order.symbol.clone(),
        order_id: order.client_id.clone().unwrap_or_default(),
        trade_id: None,
        client_id: order.client_id.clone(),
        side: order.side,
        price: order.price,
        quantity: order.quantity,
        timestamp: None,
    })
}

/// How many times the strategy's answers to outcomes are carried out in turn, as the host does.
const NESTED_ROUNDS: usize = 3;

/// A strategy run the way the host runs it: every intent it returns is answered (places taken
/// unless `refuse` says otherwise) and the answers fed back, and every fill moves the position.
pub struct SimHost<S> {
    pub strategy: S,
    pub market: Market,
    /// Whether to refuse a place: `Some(why)` refuses it.
    pub refuse: fn(&NewOrder) -> Option<&'static str>,
    /// Every intent the strategy returned, in order.
    pub sent: Vec<Intent>,
    orders: u64,
}

impl<S: StrategyAdapter> SimHost<S> {
    #[must_use]
    pub fn new(strategy: S, market: Market) -> Self {
        Self { strategy, market, refuse: |_| None, sent: Vec::new(), orders: 0 }
    }

    /// `start`, then the answers to what it asked; every intent sent, in order.
    ///
    /// # Errors
    /// The strategy's refusal to start.
    pub fn start(&mut self) -> Result<Vec<Intent>, StrategyError> {
        let first = self.strategy.start(&self.market.view())?;
        Ok(self.carry_out(first))
    }

    /// The host's warm-up before [`Self::start`]: `warmup` handed over only when the strategy
    /// asks for history, as the host does.
    pub fn warm_up(&mut self, warmup: Warmup) {
        if self.strategy.history().is_some() {
            self.strategy.warm_up(warmup);
        }
    }

    /// `event`, then the answers to what it asked. A fill moves the position first.
    pub fn send(&mut self, event: &Event) -> Vec<Intent> {
        if let Event::Fill(trade) = event {
            self.market.position = match trade.side {
                Side::Buy => self.market.position.saturating_add(trade.quantity),
                Side::Sell => self.market.position.saturating_sub(trade.quantity),
            };
        }
        let first = self.strategy.on_event(event, &self.market.view());
        self.carry_out(first)
    }

    /// `stop`; its intents are not answered.
    pub fn stop(&mut self) -> Vec<Intent> {
        let intents = self.strategy.stop(&self.market.view());
        self.sent.extend(intents.iter().cloned());
        intents
    }

    fn carry_out(&mut self, mut intents: Vec<Intent>) -> Vec<Intent> {
        let mut all = Vec::new();
        for _ in 0..=NESTED_ROUNDS {
            if intents.is_empty() {
                break;
            }
            let mut next = Vec::new();
            // The host answers cancels first, then amends, then places (not input order).
            let rank = |i: &&Intent| match i {
                Intent::Cancel(_) => 0,
                Intent::Amend(_) => 1,
                _ => 2,
            };
            let mut answered: Vec<&Intent> = intents.iter().collect();
            answered.sort_by_key(rank);
            for intent in answered {
                let refusal = match intent {
                    Intent::Place(o) => (self.refuse)(o),
                    Intent::Leg { intent: inner, .. } => match &**inner {
                        Intent::Place(o) => (self.refuse)(o),
                        _ => None,
                    },
                    Intent::Cancel(_) | Intent::Amend(_) | Intent::Flatten { .. } => None,
                    // A request is answered between events: a test sends its `Event::Answer`, and
                    // its `Event::Closed` for a close.
                    Intent::CancelAll
                    | Intent::CancelOwn
                    | Intent::Shutdown(_)
                    | Intent::Request { .. }
                    | Intent::Reply { .. }
                    | Intent::Notify(_)
                    | Intent::ClosePosition { .. }
                    | Intent::SetLimits(_)
                    | Intent::StandDown(_)
                    | Intent::Targets(_) => continue,
                    // An acked write is applied at once; one nothing waits for asks no answer.
                    Intent::Persist(write) => {
                        if let Some(id) = write.ack {
                            let mut view = self.market.view();
                            view.open_orders = None;
                            let applied = Event::Persisted { id, result: Ok(()) };
                            next.extend(self.strategy.on_event(&applied, &view));
                        }
                        continue;
                    }
                };
                let event = if let Some(why) = refusal {
                    outcome(intent, Err(why))
                } else {
                    let id = self.next_order_id();
                    outcome(intent, Ok(&id))
                };
                // The host sends open orders only with the first view.
                let mut view = self.market.view();
                view.open_orders = None;
                next.extend(self.strategy.on_event(&event, &view));
            }
            self.sent.extend(intents.iter().cloned());
            all.append(&mut intents);
            intents = next;
        }
        all
    }

    fn next_order_id(&mut self) -> String {
        self.orders = self.orders.saturating_add(1);
        format!("sim-{}", self.orders)
    }
}
