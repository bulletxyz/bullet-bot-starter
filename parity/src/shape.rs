//! The fixtures were recorded before a strategy saw exchanges: one main market on Bullet, its events and intents
//! unqualified (`Book`, `Fill`, `Place`, `CancelAll`). These map that shape, and only the shape, to the SDK's: the
//! view's market becomes Bullet's one market, a main-market event or intent names Bullet and the view's symbol. No
//! decision is touched: an intent maps to the one intent the host carried out the same way, and the fixtures stay
//! as recorded.

use std::sync::Arc;

use bb_strategy_sdk::testing::Market;
use bb_strategy_sdk::{
    BULLET, Decimal, ExchangeView, LimitsInForce, MarketFacts, MarketView, Order, OrderBook,
    Position,
};
use serde::Deserialize;
use serde_json::{Value, json};

/// A view as the fixture holds it (the bot's old `View`, owned).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ViewJson {
    now_ms: u64,
    pub(crate) symbol: String,
    book: Option<OrderBook>,
    position: Decimal,
    avg_entry: Option<Decimal>,
    realized_pnl: Decimal,
    open_orders: Option<Vec<Order>>,
    positions: Option<Vec<Position>>,
    reconnected: bool,
    venue_halt: Option<String>,
    path_stop: Option<String>,
    read_started_ms: Option<u64>,
    #[serde(default)]
    limits: Option<LimitsInForce>,
    #[serde(default)]
    stop: Option<bb_strategy_sdk::stops::StopReason>,
    market: MarketFacts,
    #[serde(default)]
    legs: Vec<Value>,
}

impl ViewJson {
    /// The same view as the SDK's: Bullet with the one market, everything per venue on Bullet.
    pub(crate) fn market(self) -> Result<Market, String> {
        if !self.legs.is_empty() {
            return Err("a view with legs: not in these fixtures".into());
        }
        let market = MarketView {
            book: self.book.map(Arc::new),
            facts: self.market,
            position: self.position,
            avg_entry: self.avg_entry,
            realized_pnl: self.realized_pnl,
            ..MarketView::new(&self.symbol)
        };
        let bullet = ExchangeView {
            open_orders: self.open_orders,
            positions: self.positions,
            reconnected: self.reconnected,
            venue_halt: self.venue_halt,
            path_stop: self.path_stop,
            read_started_ms: self.read_started_ms,
            ..ExchangeView::new(BULLET, vec![market])
        };
        Ok(Market { limits: self.limits, stop: self.stop, ..Market::of(self.now_ms, vec![bullet]) })
    }
}

fn on_bullet(intent: &Value) -> Value {
    json!({ "On": { "exchange": BULLET, "intent": intent } })
}

fn in_market(symbol: &str, event: &Value) -> Value {
    json!({ "Market": { "exchange": BULLET, "symbol": symbol, "event": event } })
}

/// A recorded intent in the SDK's shape; `symbol` is the main market's.
pub(crate) fn intent(old: &Value, symbol: &str) -> Value {
    match old {
        // The old cancels took the main market, the only one these strategies had.
        Value::String(s) if s == "CancelAll" || s == "CancelOwn" => {
            on_bullet(&json!({ s.as_str(): symbol }))
        }
        Value::Object(map) => match map.iter().next() {
            Some((k, v)) if matches!(k.as_str(), "Place" | "Cancel" | "Amend") => {
                on_bullet(&json!({ k.as_str(): v }))
            }
            _ => old.clone(),
        },
        _ => old.clone(),
    }
}

/// A recorded event in the SDK's shape; `symbol` is the main market's.
pub(crate) fn event(old: &Value, symbol: &str) -> Value {
    match old {
        Value::String(s) if s == "Book" => in_market(symbol, &json!("Book")),
        Value::Object(map) => match map.iter().next() {
            Some((k, v)) if k == "Mark" => in_market(symbol, &json!({ "Mark": v })),
            Some((k, v)) if k == "Fill" || k == "Order" => {
                let symbol = v["symbol"].as_str().unwrap_or(symbol);
                in_market(symbol, &json!({ k.as_str(): v }))
            }
            Some((k, v)) if matches!(k.as_str(), "Outcome" | "Unknown" | "Failed") => {
                let mut v = v.clone();
                v["intent"] = intent(&v["intent"], symbol);
                json!({ k.as_str(): v })
            }
            _ => old.clone(),
        },
        _ => old.clone(),
    }
}
