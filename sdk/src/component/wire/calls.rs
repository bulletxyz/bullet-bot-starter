//! Exchanges, venue calls and their answers, to and from the world's generated types.

use std::sync::Arc;

use super::{
    Uncarried, amend, amend_to_wire, book, cancel, cancel_to_wire, market, new_order,
    new_order_to_wire, order, result, side, trade, undec, undec_opt, w,
};
use crate::decimal::BadDecimal;
use crate::host::calls::{Answer, CallError, ReadQuery, VenueCall};
use crate::host::exchanges::{ExchangeView, MarketView};
use crate::strategy::adapter::{Event, ExchangeIntent, MarketEvent};
use crate::types::{AccountEquity, CancelResult, FundingSample, Position};

pub(crate) fn position(p: &w::Position) -> Result<Position, BadDecimal> {
    Ok(Position {
        symbol: p.symbol.clone(),
        side: p.side.map(side),
        size: undec(p.size)?,
        entry_price: undec(p.entry_price)?,
        unrealized_pnl: undec(p.unrealized_pnl)?,
    })
}

pub(crate) fn positions(list: &[w::Position]) -> Result<Vec<Position>, BadDecimal> {
    list.iter().map(position).collect()
}

fn market_view(m: &w::MarketView) -> Result<MarketView, BadDecimal> {
    Ok(MarketView {
        symbol: m.symbol.clone(),
        book: m.book.as_ref().map(|b| book(b).map(Arc::new)).transpose()?,
        book_at_ms: m.book_at_ms,
        mark: undec_opt(m.mark)?,
        funding_rate: undec_opt(m.funding_rate)?,
        facts: market(&m.facts)?,
        position: undec(m.position)?,
        avg_entry: undec_opt(m.avg_entry)?,
        realized_pnl: undec(m.realized_pnl)?,
    })
}

pub(crate) fn exchange_view(e: &w::ExchangeView) -> Result<ExchangeView, BadDecimal> {
    Ok(ExchangeView {
        venue: e.venue.clone(),
        markets: e.markets.iter().map(market_view).collect::<Result<_, _>>()?,
        open_orders: e
            .open_orders
            .as_ref()
            .map(|orders| orders.iter().map(order).collect::<Result<Vec<_>, _>>())
            .transpose()?,
        positions: e.positions.as_deref().map(positions).transpose()?,
        reconnected: e.reconnected,
        venue_halt: e.venue_halt.clone(),
        path_stop: e.path_stop.clone(),
        read_started_ms: e.read_started_ms,
    })
}

pub(crate) fn exchange_intent_to_wire(
    intent: ExchangeIntent,
) -> Result<w::ExchangeIntent, Uncarried> {
    Ok(match intent {
        ExchangeIntent::Place(o) => w::ExchangeIntent::Place(new_order_to_wire(o)),
        ExchangeIntent::Cancel(c) => w::ExchangeIntent::Cancel(cancel_to_wire(c)),
        ExchangeIntent::CancelAll(symbol) => w::ExchangeIntent::CancelAll(symbol),
        ExchangeIntent::Amend(a) => w::ExchangeIntent::Amend(amend_to_wire(a)),
        ExchangeIntent::Flatten(symbols) => w::ExchangeIntent::Flatten(symbols),
        ExchangeIntent::CancelOwn(_) => return Err(Uncarried("CancelOwn")),
    })
}

pub(crate) fn exchange_intent(intent: w::ExchangeIntent) -> Result<ExchangeIntent, BadDecimal> {
    Ok(match intent {
        w::ExchangeIntent::Place(o) => ExchangeIntent::Place(new_order(o)?),
        w::ExchangeIntent::Cancel(c) => ExchangeIntent::Cancel(cancel(c)),
        w::ExchangeIntent::CancelAll(symbol) => ExchangeIntent::CancelAll(symbol),
        w::ExchangeIntent::Amend(a) => ExchangeIntent::Amend(amend(a)?),
        w::ExchangeIntent::Flatten(symbols) => ExchangeIntent::Flatten(symbols),
    })
}

fn query_to_wire(q: ReadQuery) -> w::ReadQuery {
    match q {
        ReadQuery::Positions => w::ReadQuery::Positions,
        ReadQuery::OpenOrders { symbol } => w::ReadQuery::OpenOrders(symbol),
        ReadQuery::Book { symbol, depth } => w::ReadQuery::Book(w::BookQuery {
            symbol,
            depth: u32::try_from(depth).unwrap_or(u32::MAX),
        }),
        ReadQuery::AccountEquity => w::ReadQuery::AccountEquity,
        ReadQuery::MarginRatio => w::ReadQuery::MarginRatio,
        ReadQuery::FundingHistory { symbol, start_ms } => {
            w::ReadQuery::FundingHistory(w::FundingQuery { symbol, start_ms })
        }
        ReadQuery::OpenInterest { symbol } => w::ReadQuery::OpenInterest(symbol),
        ReadQuery::RefreshMetadata => w::ReadQuery::RefreshMetadata,
    }
}

fn query(q: w::ReadQuery) -> ReadQuery {
    match q {
        w::ReadQuery::Positions => ReadQuery::Positions,
        w::ReadQuery::OpenOrders(symbol) => ReadQuery::OpenOrders { symbol },
        w::ReadQuery::Book(b) => ReadQuery::Book {
            symbol: b.symbol,
            depth: usize::try_from(b.depth).unwrap_or(usize::MAX),
        },
        w::ReadQuery::AccountEquity => ReadQuery::AccountEquity,
        w::ReadQuery::MarginRatio => ReadQuery::MarginRatio,
        w::ReadQuery::FundingHistory(f) => {
            ReadQuery::FundingHistory { symbol: f.symbol, start_ms: f.start_ms }
        }
        w::ReadQuery::OpenInterest(symbol) => ReadQuery::OpenInterest { symbol },
        w::ReadQuery::RefreshMetadata => ReadQuery::RefreshMetadata,
    }
}

pub(crate) fn call_to_wire(call: VenueCall) -> w::VenueCall {
    match call {
        VenueCall::Read(q) => w::VenueCall::Read(query_to_wire(q)),
        VenueCall::Place(orders) => {
            w::VenueCall::Place(orders.into_iter().map(new_order_to_wire).collect())
        }
        VenueCall::Cancel(cancels) => {
            w::VenueCall::Cancel(cancels.into_iter().map(cancel_to_wire).collect())
        }
        VenueCall::Amend(amends) => {
            w::VenueCall::Amend(amends.into_iter().map(amend_to_wire).collect())
        }
        VenueCall::CancelAll { symbol } => w::VenueCall::CancelAll(symbol),
        VenueCall::Wait { until_ms } => w::VenueCall::Wait(until_ms),
    }
}

pub(crate) fn call(call: w::VenueCall) -> Result<VenueCall, BadDecimal> {
    Ok(match call {
        w::VenueCall::Read(q) => VenueCall::Read(query(q)),
        w::VenueCall::Place(orders) => {
            VenueCall::Place(orders.into_iter().map(new_order).collect::<Result<_, _>>()?)
        }
        w::VenueCall::Cancel(cancels) => {
            VenueCall::Cancel(cancels.into_iter().map(cancel).collect())
        }
        w::VenueCall::Amend(amends) => {
            VenueCall::Amend(amends.into_iter().map(amend).collect::<Result<_, _>>()?)
        }
        w::VenueCall::CancelAll(symbol) => VenueCall::CancelAll { symbol },
        w::VenueCall::Wait(until_ms) => VenueCall::Wait { until_ms },
    })
}

fn answer(a: w::VenueAnswer) -> Result<Answer, BadDecimal> {
    let cancel = |c: w::CancelResult| CancelResult {
        order_id: c.order_id,
        success: c.success,
        error: c.error,
    };
    Ok(match a {
        w::VenueAnswer::Positions(p) => Answer::Positions(positions(&p)?),
        w::VenueAnswer::OpenOrders(o) => {
            Answer::OpenOrders(o.iter().map(order).collect::<Result<_, _>>()?)
        }
        w::VenueAnswer::Book(b) => Answer::Book(book(&b)?),
        w::VenueAnswer::AccountEquity(e) => Answer::AccountEquity(
            e.map(|e| {
                Ok::<_, BadDecimal>(AccountEquity {
                    equity: undec(e.equity)?,
                    wallet_balance: undec(e.wallet_balance)?,
                    unrealized_pnl: undec(e.unrealized_pnl)?,
                })
            })
            .transpose()?,
        ),
        w::VenueAnswer::MarginRatio(m) => Answer::MarginRatio(undec_opt(m)?),
        w::VenueAnswer::FundingHistory(f) => Answer::FundingHistory(
            f.into_iter()
                .map(|s| Ok(FundingSample { time_ms: s.time_ms, rate_8h: undec(s.rate_8h)? }))
                .collect::<Result<_, BadDecimal>>()?,
        ),
        w::VenueAnswer::OpenInterest(o) => Answer::OpenInterest(undec_opt(o)?),
        w::VenueAnswer::Placed(r) => Answer::Placed(r.into_iter().map(result).collect()),
        w::VenueAnswer::Cancelled(c) => Answer::Cancelled(c.into_iter().map(cancel).collect()),
        w::VenueAnswer::Done => Answer::Done,
    })
}

pub(crate) fn reply(
    r: Result<w::VenueAnswer, w::CallError>,
) -> Result<Result<Answer, CallError>, BadDecimal> {
    Ok(match r {
        Ok(a) => Ok(answer(a)?),
        Err(e) => Err(CallError::new(e.message, e.never_sent, e.retryable)),
    })
}

pub(crate) fn market_event(e: w::MarketEvent) -> Result<Event, BadDecimal> {
    let event = match e.event {
        w::MarketEventKind::Book => MarketEvent::Book,
        w::MarketEventKind::Mark(m) => MarketEvent::Mark(undec(m)?),
        w::MarketEventKind::Fill(t) => MarketEvent::Fill(trade(t)?),
        w::MarketEventKind::Order(o) => MarketEvent::Order(order(&o)?),
    };
    Ok(Event::Market { exchange: e.exchange, symbol: e.symbol, event })
}
