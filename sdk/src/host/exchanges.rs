//! The exchanges a strategy trades as the host shows them ([`View::exchanges`](crate::View)): one
//! per venue it declared ([`ExchangeNeed`](crate::ExchangeNeed)), Bullet always one of them, each
//! with the markets declared on it and what the host last saw there. A strategy sends an
//! exchange's orders as [`Intent::On`](crate::Intent).

use std::sync::Arc;

use rust_decimal::Decimal;

use crate::strategy::adapter::MarketFacts;
use crate::types::{Order, OrderBook, Position, signed_position};

/// The venue every strategy trades.
pub const BULLET: &str = "bullet";

/// One market declared on an exchange: what the host knows of it now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarketView {
    pub symbol: String,
    /// The last book, and when it came (the bot's clock, ms).
    pub book: Option<Arc<OrderBook>>,
    pub book_at_ms: Option<u64>,
    /// The last positive mark price.
    pub mark: Option<Decimal>,
    /// The last live funding rate (per 8h) the venue's marks carried.
    pub funding_rate: Option<Decimal>,
    pub facts: MarketFacts,
    /// The host's inventory from fills in this market, seeded from the venue position at start.
    pub position: Decimal,
    pub avg_entry: Option<Decimal>,
    pub realized_pnl: Decimal,
}

impl MarketView {
    #[must_use]
    pub fn new(symbol: &str) -> Self {
        Self { symbol: symbol.into(), ..Self::default() }
    }

    #[must_use]
    pub fn book(&self) -> Option<&OrderBook> {
        self.book.as_deref()
    }

    /// How old the book is at `now_ms`; `None` without one.
    #[must_use]
    pub fn book_age_ms(&self, now_ms: u64) -> Option<u64> {
        self.book_at_ms.map(|t| now_ms.saturating_sub(t))
    }
}

/// One exchange (one venue): what the host knows of it now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExchangeView {
    /// The venue: what its intents and events carry.
    pub venue: String,
    /// The markets declared on it; an intent for another symbol is refused.
    pub markets: Vec<MarketView>,
    /// The strategy's orders resting on the venue, from the venue read the host just made: `Some`
    /// only for the view `start` gets and for a tick's first round when its read succeeded. `None`
    /// for every other view (a fill, an order update, a book, an outcome, a later round, `stop`)
    /// and when a tick's read failed: the list is then unknown or stale, so a strategy must not
    /// treat a level as gone.
    pub open_orders: Option<Vec<Order>>,
    /// The venue's positions from the same read as `open_orders`, with the same freshness: `Some`
    /// only for the view `start` gets and for a tick's first round when its read succeeded, `None`
    /// otherwise (unknown is never flat). A strategy sizes its closes and reconciles its inventory
    /// on this ([`Self::venue_position`]); it never reads the venue.
    pub positions: Option<Vec<Position>>,
    /// The venue stream reconnected since the strategy last saw a tick: events may have been
    /// missed, so a strategy that tracks orders or inventory reconciles them against `open_orders`
    /// and `positions` now. True on one tick's view, then false.
    pub reconnected: bool,
    /// The venue refuses orders (why), e.g. a stale oracle on its side. A strategy stops quoting,
    /// as it would under a stop of its own, until this is `None` again.
    pub venue_halt: Option<String>,
    /// The host's own order path refuses new orders (why): its stop, not the venue's. Refusals it
    /// causes are not rejections by the venue, so a strategy's reject breaker must not count them.
    pub path_stop: Option<String>,
    /// When the venue read behind `open_orders` and `positions` started (the bot clock, ms);
    /// `None` with no fresh read. A strategy that decides on what a read omits compares it with
    /// when the thing it asks about was sent, not with when the read reached it.
    pub read_started_ms: Option<u64>,
}

impl ExchangeView {
    #[must_use]
    pub fn new(venue: &str, markets: Vec<MarketView>) -> Self {
        Self { venue: venue.into(), markets, ..Self::default() }
    }

    /// The market `symbol`, when it was declared here.
    #[must_use]
    pub fn market(&self, symbol: &str) -> Option<&MarketView> {
        self.markets.iter().find(|m| m.symbol == symbol)
    }

    /// The venue's signed position in `symbol` from the last read; `None` while unknown.
    #[must_use]
    pub fn venue_position(&self, symbol: &str) -> Option<Decimal> {
        self.positions.as_deref().map(|p| signed_position(p, symbol))
    }
}
