//! What the host sends: the SDK's types to the wire, for the component glue's tests.

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use super::{dec, intent_to_wire, order_type_to_wire, side_to_wire, w};
use crate::adapter::{Event, Intent, MarketFacts, View};
use crate::calls::{Answer, CallError};
use crate::legs::LegView;
use crate::types::{Order, OrderBook, OrderResult, OrderStatus, Position, Trade};

fn status_to_wire(value: OrderStatus) -> w::OrderStatus {
    match value {
        OrderStatus::Open => w::OrderStatus::Open,
        OrderStatus::PartiallyFilled => w::OrderStatus::PartiallyFilled,
        OrderStatus::Filled => w::OrderStatus::Filled,
        OrderStatus::Cancelled => w::OrderStatus::Cancelled,
        OrderStatus::Rejected => w::OrderStatus::Rejected,
    }
}

fn book_to_wire(value: &OrderBook) -> w::OrderBook {
    let level = |(price, quantity): (&Decimal, &Decimal)| w::Level {
        price: dec(*price),
        quantity: dec(*quantity),
    };
    w::OrderBook {
        bids: value.bids.iter().map(level).collect(),
        asks: value.asks.iter().map(level).collect(),
        last_update_id: value.last_update_id,
    }
}

fn order_to_wire(o: &Order) -> w::Order {
    w::Order {
        id: o.id.clone(),
        client_id: o.client_id.clone(),
        symbol: o.symbol.clone(),
        side: side_to_wire(o.side),
        order_type: order_type_to_wire(o.order_type),
        price: dec(o.price),
        quantity: dec(o.quantity),
        filled_quantity: dec(o.filled_quantity),
        status: status_to_wire(o.status),
    }
}

fn result_to_wire(value: &OrderResult) -> w::OrderResult {
    w::OrderResult {
        order_id: value.order_id.clone(),
        client_id: value.client_id.clone(),
        success: value.success,
        error: value.error.clone(),
    }
}

fn trade_to_wire(t: &Trade) -> w::Trade {
    w::Trade {
        exchange: t.exchange.clone(),
        symbol: t.symbol.clone(),
        order_id: t.order_id.clone(),
        trade_id: t.trade_id.clone(),
        client_id: t.client_id.clone(),
        side: side_to_wire(t.side),
        price: dec(t.price),
        quantity: dec(t.quantity),
        timestamp: t.timestamp,
    }
}

fn market_to_wire(m: &MarketFacts) -> w::MarketFacts {
    w::MarketFacts {
        tick_size: m.tick_size.map(dec),
        step_size: m.step_size.map(dec),
        maker_fee_bps: m.maker_fee_bps.map(dec),
        taker_fee_bps: m.taker_fee_bps.map(dec),
        fee_tier: m.fee_tier,
        venue_name: m.venue_name.clone(),
    }
}

fn position_to_wire(p: &Position) -> w::Position {
    w::Position {
        symbol: p.symbol.clone(),
        side: p.side.map(side_to_wire),
        size: dec(p.size),
        entry_price: dec(p.entry_price),
        unrealized_pnl: dec(p.unrealized_pnl),
    }
}

fn to_list<T, W>(map: &BTreeMap<String, T>, f: impl Fn(&T) -> W) -> Vec<(String, W)> {
    map.iter().map(|(s, v)| (s.clone(), f(v))).collect()
}

fn leg_view_to_wire(l: &LegView) -> w::LegView {
    w::LegView {
        id: l.id.clone(),
        venue: l.venue.clone(),
        symbols: l.symbols.clone(),
        books: to_list(&l.books, book_to_wire),
        book_at_ms: l.book_at_ms.iter().map(|(s, t)| (s.clone(), *t)).collect(),
        marks: to_list(&l.marks, |m| dec(*m)),
        funding_rates: to_list(&l.funding_rates, |r| dec(*r)),
        positions: l.positions.as_ref().map(|p| p.iter().map(position_to_wire).collect()),
        market: to_list(&l.market, market_to_wire),
        reconnected: l.reconnected,
    }
}

fn answer_to_wire(a: &Answer) -> w::VenueAnswer {
    match a {
        Answer::Positions(p) => w::VenueAnswer::Positions(p.iter().map(position_to_wire).collect()),
        Answer::OpenOrders(o) => w::VenueAnswer::OpenOrders(o.iter().map(order_to_wire).collect()),
        Answer::Book(b) => w::VenueAnswer::Book(book_to_wire(b)),
        Answer::AccountEquity(e) => {
            w::VenueAnswer::AccountEquity(e.as_ref().map(|e| w::AccountEquity {
                equity: dec(e.equity),
                wallet_balance: dec(e.wallet_balance),
                unrealized_pnl: dec(e.unrealized_pnl),
            }))
        }
        Answer::MarginRatio(m) => w::VenueAnswer::MarginRatio(m.map(dec)),
        Answer::FundingHistory(f) => w::VenueAnswer::FundingHistory(
            f.iter()
                .map(|s| w::FundingSample { time_ms: s.time_ms, rate_8h: dec(s.rate_8h) })
                .collect(),
        ),
        Answer::OpenInterest(o) => w::VenueAnswer::OpenInterest(o.map(dec)),
        Answer::Placed(r) => w::VenueAnswer::Placed(r.iter().map(result_to_wire).collect()),
        Answer::Cancelled(c) => w::VenueAnswer::Cancelled(
            c.iter()
                .map(|c| w::CancelResult {
                    order_id: c.order_id.clone(),
                    success: c.success,
                    error: c.error.clone(),
                })
                .collect(),
        ),
        Answer::Done => w::VenueAnswer::Done,
    }
}

fn reply_to_wire(r: &Result<Answer, CallError>) -> Result<w::VenueAnswer, w::CallError> {
    match r {
        Ok(a) => Ok(answer_to_wire(a)),
        Err(e) => Err(w::CallError {
            message: e.message.clone(),
            never_sent: e.never_sent,
            retryable: e.retryable,
        }),
    }
}

/// A leg's event as it crosses; `None` for what the host never sends on a leg.
fn leg_event_to_wire(leg: &str, symbol: &str, event: &Event) -> Option<w::LegEvent> {
    let kind = match event {
        Event::Book => w::LegEventKind::Book,
        Event::Mark(m) => w::LegEventKind::Mark(dec(*m)),
        Event::Fill(t) => w::LegEventKind::Fill(trade_to_wire(t)),
        Event::Order(o) => w::LegEventKind::Order(order_to_wire(o)),
        Event::Tick
        | Event::Outcome { .. }
        | Event::Leg { .. }
        | Event::Answer { .. }
        | Event::Unknown { .. } => return None,
    };
    Some(w::LegEvent { leg: leg.into(), symbol: symbol.into(), event: kind })
}

pub(crate) fn view(view: &View<'_>) -> w::View {
    w::View {
        now_ms: view.now_ms,
        symbol: view.symbol.to_owned(),
        book: view.book.map(book_to_wire),
        position: dec(view.position),
        avg_entry: view.avg_entry.map(dec),
        realized_pnl: dec(view.realized_pnl),
        open_orders: view.open_orders.map(|orders| orders.iter().map(order_to_wire).collect()),
        positions: view.positions.map(|p| p.iter().map(position_to_wire).collect()),
        reconnected: view.reconnected,
        venue_halt: view.venue_halt.map(str::to_owned),
        market: market_to_wire(&view.market),
        legs: view.legs.iter().map(leg_view_to_wire).collect(),
        path_stop: view.path_stop.map(str::to_owned),
        read_started_ms: view.read_started_ms,
    }
}

fn echo(intent: &Intent) -> w::Intent {
    intent_to_wire(intent.clone()).unwrap_or(w::Intent::CancelAll)
}

fn outcome(intent: &Intent, result: &OrderResult) -> w::Outcome {
    w::Outcome { intent: echo(intent), result: result_to_wire(result) }
}

pub(crate) fn event(event: &Event) -> w::Event {
    match event {
        Event::Book => w::Event::Book,
        Event::Mark(m) => w::Event::Mark(dec(*m)),
        Event::Fill(t) => w::Event::Fill(trade_to_wire(t)),
        Event::Order(o) => w::Event::Order(order_to_wire(o)),
        Event::Tick => w::Event::Tick,
        Event::Outcome { intent, result } => w::Event::Outcome(outcome(intent, result)),
        Event::Leg { leg, symbol, event } => {
            leg_event_to_wire(leg, symbol, event).map_or(w::Event::Tick, w::Event::Leg)
        }
        Event::Answer { id, reply } => {
            w::Event::Answer(w::Answer { id: *id, reply: reply_to_wire(reply) })
        }
        Event::Unknown { intent, error } => {
            w::Event::Unknown(w::Unknown { intent: echo(intent), error: error.clone() })
        }
    }
}
