//! The order and market types a strategy reads and returns. They mirror bb-core's (`Side`,
//! `OrderBook`, `NewOrder`, `Trade`, ...) field for field; the host converts at the boundary. The
//! later SDK split moves bb-core onto these, so the two copies become one.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    #[must_use]
    pub fn opposite(self) -> Self {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrderType {
    Limit,
    PostOnly,
    Market,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub enum OrderStatus {
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
}

/// One price level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub price: Decimal,
    pub quantity: Decimal,
}

/// A book: price to quantity on each side.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderBook {
    pub bids: BTreeMap<Decimal, Decimal>,
    pub asks: BTreeMap<Decimal, Decimal>,
    pub last_update_id: u64,
}

impl OrderBook {
    #[must_use]
    pub fn best_bid(&self) -> Option<Level> {
        self.bids.iter().next_back().map(|(&price, &quantity)| Level { price, quantity })
    }

    #[must_use]
    pub fn best_ask(&self) -> Option<Level> {
        self.asks.iter().next().map(|(&price, &quantity)| Level { price, quantity })
    }

    /// The mid of the best bid and ask; `None` when a side is empty.
    #[must_use]
    pub fn midpoint(&self) -> Option<Decimal> {
        let (bid, ask) = (self.best_bid()?, self.best_ask()?);
        bid.price.checked_add(ask.price)?.checked_div(Decimal::TWO)
    }
}

/// One of the strategy's orders as the venue lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    pub id: String,
    pub client_id: Option<String>,
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    pub price: Decimal,
    pub quantity: Decimal,
    pub filled_quantity: Decimal,
    pub status: OrderStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewOrder {
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    pub price: Decimal,
    pub quantity: Decimal,
    pub client_id: Option<String>,
    pub reduce_only: bool,
    /// The stop (and take-profit) the venue holds for the position once it fills (world 0.4.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protect: Option<Protection>,
}

/// The stop (and take-profit) the venue holds for a position: the price its mark must cross
/// (below a long, above a short for the stop), when the venue closes the whole position. It never
/// opens one; a venue that holds none refuses the order; paper sends none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Protection {
    pub stop: Option<Decimal>,
    pub take_profit: Option<Decimal>,
}

/// A cancel by venue order id, or by client id when `order_id` is empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelOrder {
    pub symbol: String,
    pub order_id: String,
    pub client_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmendOrder {
    pub cancel: CancelOrder,
    pub new_order: NewOrder,
}

/// What the venue (or the host's gate) answered for one intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderResult {
    /// The venue's order id when the place is confirmed; `None`: unknown yet.
    pub order_id: Option<String>,
    pub client_id: Option<String>,
    pub success: bool,
    pub error: Option<String>,
}

/// A fill of one of the bot's orders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trade {
    pub exchange: String,
    pub symbol: String,
    pub order_id: String,
    pub trade_id: Option<String>,
    pub client_id: Option<String>,
    pub side: Side,
    pub price: Decimal,
    pub quantity: Decimal,
    /// Unix milliseconds, when the venue reports it.
    pub timestamp: Option<u64>,
}

/// What the venue answered for one cancel of a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelResult {
    pub order_id: String,
    pub success: bool,
    pub error: Option<String>,
}

/// One of the venue's positions; `side` is `None` when flat.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub side: Option<Side>,
    pub size: Decimal,
    pub entry_price: Decimal,
    pub unrealized_pnl: Decimal,
}

/// `symbol`'s signed size in a positions read: short negative, absent flat.
#[must_use]
pub fn signed_position(positions: &[Position], symbol: &str) -> Decimal {
    positions.iter().find(|p| p.symbol == symbol).map_or(Decimal::ZERO, |p| match p.side {
        Some(Side::Buy) => p.size,
        Some(Side::Sell) => Decimal::ZERO.saturating_sub(p.size),
        None => Decimal::ZERO,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountEquity {
    pub equity: Decimal,
    pub wallet_balance: Decimal,
    pub unrealized_pnl: Decimal,
}

/// One settled funding payment, at the 8h-equivalent rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FundingSample {
    pub time_ms: i64,
    pub rate_8h: Decimal,
}
