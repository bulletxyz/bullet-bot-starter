//! The SDK's types to and from the `bb:strategy` world's generated ones.

use rust_decimal::Decimal;

/// The world's generated types (`bb:strategy/adapter`).
pub use crate::__bindings::exports::bb::strategy::adapter as w;
use crate::decimal::{self, BadDecimal, WireDecimal};
use crate::host::exchanges::ExchangeView;
use crate::strategy::adapter::{Event, FieldValue, Intent, MarketFacts, ReportField, View};
use crate::types::{
    AmendOrder, CancelOrder, NewOrder, Order, OrderBook, OrderResult, OrderStatus, OrderType,
    Protection, Side, Trade,
};

mod calls;
pub(crate) mod history;

pub(crate) fn dec(value: Decimal) -> w::Decimal {
    let WireDecimal { hi, lo, scale } = decimal::encode(value);
    w::Decimal { hi, lo, scale }
}

pub(crate) fn field_to_wire(field: ReportField) -> w::ReportField {
    let value = match field.value {
        FieldValue::Number(n) => w::FieldValue::Number(dec(n)),
        FieldValue::Text(t) => w::FieldValue::Text(t),
        FieldValue::Flag(f) => w::FieldValue::Flag(f),
    };
    w::ReportField { name: field.name, value, unit: field.unit }
}

pub(crate) fn undec(value: w::Decimal) -> Result<Decimal, BadDecimal> {
    decimal::decode(WireDecimal { hi: value.hi, lo: value.lo, scale: value.scale })
}

pub(crate) fn undec_opt(value: Option<w::Decimal>) -> Result<Option<Decimal>, BadDecimal> {
    value.map(undec).transpose()
}

pub(crate) fn side(value: w::Side) -> Side {
    match value {
        w::Side::Buy => Side::Buy,
        w::Side::Sell => Side::Sell,
    }
}

pub(crate) fn side_to_wire(value: Side) -> w::Side {
    match value {
        Side::Buy => w::Side::Buy,
        Side::Sell => w::Side::Sell,
    }
}

fn order_type(value: w::OrderType) -> OrderType {
    match value {
        w::OrderType::Limit => OrderType::Limit,
        w::OrderType::PostOnly => OrderType::PostOnly,
        w::OrderType::Market => OrderType::Market,
    }
}

pub(crate) fn order_type_to_wire(value: OrderType) -> w::OrderType {
    match value {
        OrderType::Limit => w::OrderType::Limit,
        OrderType::PostOnly => w::OrderType::PostOnly,
        OrderType::Market => w::OrderType::Market,
    }
}

fn status(value: w::OrderStatus) -> OrderStatus {
    match value {
        w::OrderStatus::Open => OrderStatus::Open,
        w::OrderStatus::PartiallyFilled => OrderStatus::PartiallyFilled,
        w::OrderStatus::Filled => OrderStatus::Filled,
        w::OrderStatus::Cancelled => OrderStatus::Cancelled,
        w::OrderStatus::Rejected => OrderStatus::Rejected,
    }
}

fn levels(levels: &[w::Level]) -> Result<std::collections::BTreeMap<Decimal, Decimal>, BadDecimal> {
    levels.iter().map(|l| Ok((undec(l.price)?, undec(l.quantity)?))).collect()
}

pub(crate) fn book(value: &w::OrderBook) -> Result<OrderBook, BadDecimal> {
    Ok(OrderBook {
        bids: levels(&value.bids)?,
        asks: levels(&value.asks)?,
        last_update_id: value.last_update_id,
    })
}

pub(crate) fn order(value: &w::Order) -> Result<Order, BadDecimal> {
    Ok(Order {
        id: value.id.clone(),
        client_id: value.client_id.clone(),
        symbol: value.symbol.clone(),
        side: side(value.side),
        order_type: order_type(value.order_type),
        price: undec(value.price)?,
        quantity: undec(value.quantity)?,
        filled_quantity: undec(value.filled_quantity)?,
        status: status(value.status),
    })
}

pub(crate) fn new_order(value: w::NewOrder) -> Result<NewOrder, BadDecimal> {
    Ok(NewOrder {
        symbol: value.symbol,
        side: side(value.side),
        order_type: order_type(value.order_type),
        price: undec(value.price)?,
        quantity: undec(value.quantity)?,
        client_id: value.client_id,
        reduce_only: value.reduce_only,
        protect: value.protect.map(protection).transpose()?,
    })
}

fn protection(value: w::Protection) -> Result<Protection, BadDecimal> {
    Ok(Protection {
        stop: value.stop.map(undec).transpose()?,
        take_profit: value.take_profit.map(undec).transpose()?,
    })
}

pub(crate) fn new_order_to_wire(value: NewOrder) -> w::NewOrder {
    w::NewOrder {
        symbol: value.symbol,
        side: side_to_wire(value.side),
        order_type: order_type_to_wire(value.order_type),
        price: dec(value.price),
        quantity: dec(value.quantity),
        client_id: value.client_id,
        reduce_only: value.reduce_only,
        protect: value
            .protect
            .map(|p| w::Protection { stop: p.stop.map(dec), take_profit: p.take_profit.map(dec) }),
    }
}

pub(crate) fn cancel(value: w::CancelOrder) -> CancelOrder {
    CancelOrder { symbol: value.symbol, order_id: value.order_id, client_id: value.client_id }
}

pub(crate) fn cancel_to_wire(value: CancelOrder) -> w::CancelOrder {
    w::CancelOrder { symbol: value.symbol, order_id: value.order_id, client_id: value.client_id }
}

pub(crate) fn amend(value: w::AmendOrder) -> Result<AmendOrder, BadDecimal> {
    Ok(AmendOrder { cancel: cancel(value.cancel), new_order: new_order(value.new_order)? })
}

pub(crate) fn amend_to_wire(value: AmendOrder) -> w::AmendOrder {
    w::AmendOrder {
        cancel: cancel_to_wire(value.cancel),
        new_order: new_order_to_wire(value.new_order),
    }
}

pub(crate) fn result(value: w::OrderResult) -> OrderResult {
    OrderResult {
        order_id: value.order_id,
        client_id: value.client_id,
        success: value.success,
        error: value.error,
    }
}

/// An intent the `bb:strategy@0.6.0` world has no case for: a guest that returns one is faulted by
/// the host (it traps), never silently dropped: what it asked for would not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Uncarried(pub(crate) &'static str);

impl std::fmt::Display for Uncarried {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the bb:strategy@0.6.0 world cannot carry Intent::{}", self.0)
    }
}

/// `Err` for an intent this world has no case for.
pub(crate) fn intent_to_wire(value: Intent) -> Result<w::Intent, Uncarried> {
    Ok(match value {
        Intent::On { exchange, intent } => w::Intent::On(w::OnExchange {
            exchange,
            intent: calls::exchange_intent_to_wire(intent)?,
        }),
        Intent::Shutdown(why) => w::Intent::Shutdown(why),
        Intent::Request { id, exchange, call } => {
            w::Intent::Request(w::Request { id, exchange, call: calls::call_to_wire(call) })
        }
        other => return Err(uncarried(&other)),
    })
}

/// The intent the world has no case for, by name.
pub(crate) fn uncarried(intent: &Intent) -> Uncarried {
    Uncarried(match intent {
        Intent::Reply { .. } => "Reply",
        Intent::Persist(_) => "Persist",
        Intent::Notify(_) => "Notify",
        Intent::ClosePosition { .. } => "ClosePosition",
        Intent::SetLimits(_) => "SetLimits",
        Intent::StandDown(_) => "StandDown",
        Intent::Targets(_) => "Targets",
        Intent::On { .. } | Intent::Shutdown(_) | Intent::Request { .. } => {
            "a case the world carries"
        }
    })
}

pub(crate) fn intent_from_wire(value: w::Intent) -> Result<Intent, BadDecimal> {
    Ok(match value {
        w::Intent::On(on) => {
            Intent::On { exchange: on.exchange, intent: calls::exchange_intent(on.intent)? }
        }
        w::Intent::Shutdown(why) => Intent::Shutdown(why),
        w::Intent::Request(r) => {
            Intent::Request { id: r.id, exchange: r.exchange, call: calls::call(r.call)? }
        }
    })
}

pub(crate) fn trade(value: w::Trade) -> Result<Trade, BadDecimal> {
    Ok(Trade {
        exchange: value.exchange,
        symbol: value.symbol,
        order_id: value.order_id,
        trade_id: value.trade_id,
        client_id: value.client_id,
        side: side(value.side),
        price: undec(value.price)?,
        quantity: undec(value.quantity)?,
        timestamp: value.timestamp,
    })
}

pub(crate) fn market(m: &w::MarketFacts) -> Result<MarketFacts, BadDecimal> {
    Ok(MarketFacts {
        tick_size: undec_opt(m.tick_size)?,
        step_size: undec_opt(m.step_size)?,
        maker_fee_bps: undec_opt(m.maker_fee_bps)?,
        taker_fee_bps: undec_opt(m.taker_fee_bps)?,
        fee_tier: m.fee_tier,
        venue_name: m.venue_name.clone(),
    })
}

pub(crate) fn event_from_wire(value: w::Event) -> Result<Event, BadDecimal> {
    Ok(match value {
        w::Event::Market(e) => calls::market_event(e)?,
        w::Event::Tick => Event::Tick,
        w::Event::Outcome(o) => {
            Event::Outcome { intent: intent_from_wire(o.intent)?, result: result(o.result) }
        }
        w::Event::Answer(a) => Event::Answer { id: a.id, reply: calls::reply(a.reply)? },
        w::Event::Unknown(u) => {
            Event::Unknown { intent: intent_from_wire(u.intent)?, error: u.error }
        }
    })
}

/// A view decoded from the wire, owning what [`View`] borrows.
#[derive(Debug, Clone)]
pub(crate) struct OwnedView {
    now_ms: u64,
    exchanges: Vec<ExchangeView>,
}

impl OwnedView {
    pub(crate) fn from_wire(value: &w::View) -> Result<Self, BadDecimal> {
        Ok(Self {
            now_ms: value.now_ms,
            exchanges: value
                .exchanges
                .iter()
                .map(calls::exchange_view)
                .collect::<Result<_, _>>()?,
        })
    }

    pub(crate) fn view(&self) -> View<'_> {
        View { now_ms: self.now_ms, exchanges: &self.exchanges, limits: None, stop: None }
    }
}

/// The host's side of the wire, for the glue's tests (the host's own conversion mirrors this).
#[cfg(test)]
pub(crate) mod host;

fn range_to_wire(range: crate::strategy::describe::Range) -> w::Range {
    w::Range { min: range.min.map(dec), max: range.max.map(dec) }
}

/// A descriptor as it crosses.
pub(crate) fn descriptor_to_wire(
    name: &str,
    version: &str,
    d: crate::strategy::describe::Descriptor,
) -> w::Descriptor {
    use crate::strategy::describe::{SettingKind, Tier};
    let setting = |s: crate::strategy::describe::SettingSpec| w::Setting {
        path: s.path,
        kind: match s.kind {
            SettingKind::Decimal => w::SettingKind::Decimal,
            SettingKind::Int => w::SettingKind::Int,
            SettingKind::Flag => w::SettingKind::Flag,
            SettingKind::Choice(c) => w::SettingKind::Choice(c),
            SettingKind::Text(t) => w::SettingKind::Text(t),
            SettingKind::Words(w) => w::SettingKind::Words(w),
        },
        default: dec(s.default),
        guarded: range_to_wire(s.guarded),
        hard: range_to_wire(s.hard),
        help: s.help,
        tier: match s.tier {
            Tier::Tune => w::Tier::Tune,
            Tier::Structural => w::Tier::Structural,
            Tier::Limit => w::Tier::Limit,
            Tier::Guard => w::Tier::Guard,
        },
        loosening: match s.loosening {
            crate::strategy::describe::Loosening::None => w::Loosening::None,
            crate::strategy::describe::Loosening::Up => w::Loosening::Up,
            crate::strategy::describe::Loosening::Down => w::Loosening::Down,
        },
        required: s.required,
    };
    w::Descriptor {
        name: name.into(),
        version: version.into(),
        summary: d.summary,
        settings: d.settings.into_iter().map(setting).collect(),
        settings_schema: serde_json::to_string(&d.settings_schema)
            .unwrap_or_else(|_| "true".into()),
        exchanges: d.exchanges.into_iter().map(|e| w::ExchangeNeed { venue: e.venue }).collect(),
    }
}
