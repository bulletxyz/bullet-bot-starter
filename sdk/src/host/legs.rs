//! A strategy's other legs as the host shows them ([`View::legs`](crate::View)): each a venue
//! account and the symbols the strategy declared on it ([`LegNeed`](crate::LegNeed)), with what
//! the host last saw there. A strategy sends a leg's orders as [`Intent::Leg`](crate::Intent).

use std::collections::BTreeMap;
use std::sync::Arc;

use rust_decimal::Decimal;

use crate::strategy::adapter::MarketFacts;
use crate::types::{OrderBook, Position, signed_position};

/// The leg the host was built over (its one symbol): `Intent::Leg` names it with this id; a plain
/// intent is on it too.
pub const TRADE_LEG: &str = "trade";

/// One leg: what the host knows of it now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegView {
    /// The strategy's name for it (`hedge`): what its intents and events carry.
    pub id: String,
    /// The venue it trades.
    pub venue: String,
    /// The symbols declared on it; an intent for another one is refused.
    pub symbols: Vec<String>,
    /// The last book of each symbol, and when it came (the bot's clock, ms).
    pub books: BTreeMap<String, Arc<OrderBook>>,
    pub book_at_ms: BTreeMap<String, u64>,
    /// The last positive mark price of each symbol.
    pub marks: BTreeMap<String, Decimal>,
    /// The last live funding rate (per 8h) the venue's marks carried for each symbol.
    pub funding_rates: BTreeMap<String, Decimal>,
    /// The venue's positions as the last read of this leg found them: `None` when that read
    /// failed, and from a fill on the leg until the next read. Unknown is never flat.
    pub positions: Option<Vec<Position>>,
    /// The venue's facts per symbol.
    pub market: BTreeMap<String, MarketFacts>,
    /// The leg's connection reconnected since the tick before: what came meanwhile may have been
    /// missed. True for the tick that read it only.
    pub reconnected: bool,
}

impl LegView {
    #[must_use]
    pub fn new(id: &str, venue: &str, symbols: Vec<String>) -> Self {
        Self { id: id.into(), venue: venue.into(), symbols, ..Self::default() }
    }

    /// Whether `symbol` was declared on this leg.
    #[must_use]
    pub fn holds(&self, symbol: &str) -> bool {
        self.symbols.iter().any(|s| s == symbol)
    }

    /// The venue's signed position in `symbol` from the last read; `None` while unknown.
    #[must_use]
    pub fn position(&self, symbol: &str) -> Option<Decimal> {
        self.positions.as_deref().map(|p| signed_position(p, symbol))
    }

    #[must_use]
    pub fn book(&self, symbol: &str) -> Option<&OrderBook> {
        self.books.get(symbol).map(AsRef::as_ref)
    }

    /// How old `symbol`'s book is at `now_ms`; `None` without one.
    #[must_use]
    pub fn book_age_ms(&self, symbol: &str, now_ms: u64) -> Option<u64> {
        self.book_at_ms.get(symbol).map(|t| now_ms.saturating_sub(*t))
    }
}
