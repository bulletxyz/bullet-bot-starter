//! Helpers for a strategy's own tests (feature `testing`): the exchanges to build views from, and
//! [`SimHost`], a small simulated host that runs a strategy the way bb-strategy's `StrategyHost`
//! does (start, events, each intent's outcome fed back, at most 3 nested rounds), without a venue.
//! Golden replays through the real host live in the host's repository, not here.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::Decimal;
use crate::history::candles::{Candle, CandleInterval, Warmup};
use crate::host::exchanges::{BULLET, ExchangeView, MarketView};
use crate::strategy::adapter::{
    Event, ExchangeIntent, Intent, MarketEvent, StrategyAdapter, StrategyError, View,
};
use crate::types::{NewOrder, OrderBook, OrderResult, Side, Trade};

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

/// `symbol` with a book of `bid` / `ask` (10 each), flat.
#[must_use]
pub fn market(symbol: &str, bid: &str, ask: &str) -> MarketView {
    MarketView { book: Some(Arc::new(book(bid, ask, "10"))), ..MarketView::new(symbol) }
}

/// `venue` with `markets`, no orders listed.
#[must_use]
pub fn exchange(venue: &str, markets: Vec<MarketView>) -> ExchangeView {
    ExchangeView { open_orders: Some(Vec::new()), ..ExchangeView::new(venue, markets) }
}

/// What a test shows a strategy, owned; [`Market::view`] borrows it as the host would.
#[derive(Debug, Clone, Default)]
pub struct Market {
    pub now_ms: u64,
    pub exchanges: Vec<ExchangeView>,
    pub limits: Option<crate::LimitsInForce>,
    pub stop: Option<crate::stops::StopReason>,
}

impl Market {
    /// `symbol` on Bullet with a book of `bid` / `ask`, flat, no orders listed, at `now_ms`.
    #[must_use]
    pub fn new(symbol: &str, bid: &str, ask: &str, now_ms: u64) -> Self {
        Self::of(now_ms, vec![exchange(BULLET, vec![market(symbol, bid, ask)])])
    }

    /// Several markets on several exchanges (see [`exchange`] and [`market`]), at `now_ms`.
    #[must_use]
    pub fn of(now_ms: u64, exchanges: Vec<ExchangeView>) -> Self {
        Self { now_ms, exchanges, ..Self::default() }
    }

    #[must_use]
    pub fn view(&self) -> View<'_> {
        self.view_over(&self.exchanges)
    }

    pub fn exchange_mut(&mut self, venue: &str) -> Option<&mut ExchangeView> {
        self.exchanges.iter_mut().find(|e| e.venue == venue)
    }

    pub fn market_mut(&mut self, venue: &str, symbol: &str) -> Option<&mut MarketView> {
        self.exchange_mut(venue)?.markets.iter_mut().find(|m| m.symbol == symbol)
    }

    /// Bullet, the exchange [`Market::new`] makes.
    pub fn bullet_mut(&mut self) -> Option<&mut ExchangeView> {
        self.exchange_mut(BULLET)
    }

    /// Bullet's first market: the one [`Market::new`] makes.
    pub fn main_mut(&mut self) -> Option<&mut MarketView> {
        self.bullet_mut()?.markets.first_mut()
    }

    /// The book of [`Self::main_mut`]'s market (none: no book).
    pub fn set_book(&mut self, book: Option<OrderBook>) {
        if let Some(market) = self.main_mut() {
            market.book = book.map(Arc::new);
        }
    }

    fn view_over<'a>(&'a self, exchanges: &'a [ExchangeView]) -> View<'a> {
        View {
            now_ms: self.now_ms,
            exchanges,
            limits: self.limits.clone(),
            stop: self.stop.as_ref(),
        }
    }

    /// The exchanges as a later round sees them: the host sends open orders only with the first view.
    fn unlisted(&self) -> Vec<ExchangeView> {
        let unlist = |e: &ExchangeView| ExchangeView { open_orders: None, ..e.clone() };
        self.exchanges.iter().map(unlist).collect()
    }
}

/// The places among `intents`, on every exchange.
#[must_use]
pub fn placed(intents: &[Intent]) -> Vec<&NewOrder> {
    intents
        .iter()
        .filter_map(|i| match i {
            Intent::On { intent: ExchangeIntent::Place(o), .. } => Some(o),
            _ => None,
        })
        .collect()
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
        Intent::On { intent, .. } => match intent {
            ExchangeIntent::Place(o) => o.client_id.clone(),
            ExchangeIntent::Amend(a) => a.new_order.client_id.clone(),
            ExchangeIntent::Cancel(c) => c.client_id.clone(),
            ExchangeIntent::CancelAll(_)
            | ExchangeIntent::CancelOwn(_)
            | ExchangeIntent::Flatten(_) => None,
        },
        Intent::Shutdown(_)
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

/// `event` in the market `symbol` on Bullet.
#[must_use]
pub fn on_bullet(symbol: &str, event: MarketEvent) -> Event {
    Event::Market { exchange: BULLET.into(), symbol: symbol.into(), event }
}

/// A full fill of `order` on Bullet.
#[must_use]
pub fn fill_of(order: &NewOrder) -> Event {
    fill_on(BULLET, order)
}

/// A full fill of `order` on `exchange`.
#[must_use]
pub fn fill_on(exchange: &str, order: &NewOrder) -> Event {
    let trade = Trade {
        exchange: exchange.into(),
        symbol: order.symbol.clone(),
        order_id: order.client_id.clone().unwrap_or_default(),
        trade_id: None,
        client_id: order.client_id.clone(),
        side: order.side,
        price: order.price,
        quantity: order.quantity,
        timestamp: None,
    };
    Event::Market {
        exchange: exchange.into(),
        symbol: order.symbol.clone(),
        event: MarketEvent::Fill(trade),
    }
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

    /// `event`, then the answers to what it asked. A fill moves its market's position first.
    pub fn send(&mut self, event: &Event) -> Vec<Intent> {
        if let Event::Market { exchange, symbol, event: MarketEvent::Fill(trade) } = event
            && let Some(market) = self.market.market_mut(exchange, symbol)
        {
            market.position = match trade.side {
                Side::Buy => market.position.saturating_add(trade.quantity),
                Side::Sell => market.position.saturating_sub(trade.quantity),
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
                Intent::On { intent: ExchangeIntent::Cancel(_), .. } => 0,
                Intent::On { intent: ExchangeIntent::Amend(_), .. } => 1,
                _ => 2,
            };
            let mut answered: Vec<&Intent> = intents.iter().collect();
            answered.sort_by_key(rank);
            for intent in answered {
                let refusal = match intent {
                    Intent::On { intent, .. } => match intent {
                        ExchangeIntent::Place(o) => (self.refuse)(o),
                        ExchangeIntent::Cancel(_)
                        | ExchangeIntent::Amend(_)
                        | ExchangeIntent::Flatten(_) => None,
                        ExchangeIntent::CancelAll(_) | ExchangeIntent::CancelOwn(_) => continue,
                    },
                    // A request is answered between events: a test sends its `Event::Answer`, and
                    // its `Event::Closed` for a close.
                    Intent::Shutdown(_)
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
                            let unlisted = self.market.unlisted();
                            let applied = Event::Persisted { id, result: Ok(()) };
                            next.extend(
                                self.strategy.on_event(&applied, &self.market.view_over(&unlisted)),
                            );
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
                let unlisted = self.market.unlisted();
                next.extend(self.strategy.on_event(&event, &self.market.view_over(&unlisted)));
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
