//! The order and market types a strategy reads and returns. They are bb-core's (`Side`,
//! `OrderBook`, `NewOrder`, `Trade`, ...) field for field: the bot re-exports these, one copy.

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

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Side::Buy => "buy",
            Side::Sell => "sell",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrderType {
    Limit,
    PostOnly,
    Market,
}

impl std::fmt::Display for OrderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            OrderType::Limit => "limit",
            OrderType::PostOnly => "post_only",
            OrderType::Market => "market",
        })
    }
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

    /// The best ask less the best bid; `None` when a side is empty or it overflows.
    #[must_use]
    pub fn spread(&self) -> Option<Decimal> {
        let (bid, ask) = (self.best_bid()?, self.best_ask()?);
        ask.price.checked_sub(bid.price)
    }

    /// Would a `PostOnly` order at (`side`, `price`) cross the top of book?
    ///
    /// `true` when a venue would reject the order as in-cross: buys at or above the best ask, sells
    /// at or below the best bid. When the relevant side of the book is empty it is `false` (no
    /// info = don't block placement; the venue is authoritative).
    #[must_use]
    pub fn would_cross(&self, side: Side, price: Decimal) -> bool {
        match side {
            Side::Buy => self.best_ask().is_some_and(|ask| price >= ask.price),
            Side::Sell => self.best_bid().is_some_and(|bid| price <= bid.price),
        }
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

impl Protection {
    #[must_use]
    pub fn stop_at(price: Decimal) -> Self {
        Self { stop: Some(price), take_profit: None }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stop.is_none() && self.take_profit.is_none()
    }

    /// Why this protection cannot guard a position on `side` (the entry's side) at `price`: a stop
    /// must be on the losing side of it and a take-profit on the winning side, both above zero.
    /// `None` when it can.
    #[must_use]
    pub fn misplaced(&self, side: Side, price: Decimal) -> Option<String> {
        if self.is_empty() {
            return Some("a protection needs a stop or a take-profit".into());
        }
        // Below the price for a buy, above it for a sell.
        let below = |x: Decimal| if side == Side::Buy { x < price } else { x > price };
        let (losing, winning) = match side {
            Side::Buy => ("below", "above"),
            Side::Sell => ("above", "below"),
        };
        if let Some(stop) = self.stop
            && (stop <= Decimal::ZERO || !below(stop))
        {
            return Some(format!("the stop {stop} must be {losing} {price} for a {side}"));
        }
        if let Some(tp) = self.take_profit
            && (tp <= Decimal::ZERO || below(tp) || tp == price)
        {
            return Some(format!("the take-profit {tp} must be {winning} {price} for a {side}"));
        }
        None
    }
}

/// Which leg of a protection a held stop is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopKind {
    Stop,
    TakeProfit,
}

/// A stop the venue holds, as a read of it shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VenueStop {
    /// The venue's id (its own namespace: never an order id).
    pub id: String,
    pub symbol: String,
    /// The side it closes with: a sell guards a long.
    pub side: Side,
    pub kind: StopKind,
    pub trigger_price: Decimal,
    /// `None`: the whole position when it fires; else that size.
    pub size: Option<Decimal>,
    /// This bot made it (the adapter recognises its own); anything else was the owner's.
    pub by_bot: bool,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountEquity {
    pub equity: Decimal,
    pub wallet_balance: Decimal,
    pub unrealized_pnl: Decimal,
}

/// What a cancel-and-place came back with: one result per cancel and one per new order, each in
/// the order sent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplaceResult {
    pub cancels: Vec<CancelResult>,
    pub placed: Vec<OrderResult>,
}

/// One asset's balance on the venue.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Balance {
    pub asset: String,
    pub available: Decimal,
    pub total: Decimal,
}

/// One settled funding payment, at the 8h-equivalent rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FundingSample {
    pub time_ms: i64,
    pub rate_8h: Decimal,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
