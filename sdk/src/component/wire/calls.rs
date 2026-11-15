//! Legs, venue calls and their answers, to and from the world's generated types.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{
    Uncarried, amend, amend_to_wire, book, cancel, cancel_to_wire, market, new_order,
    new_order_to_wire, order, result, side, trade, uncarried, undec, undec_opt, w,
};
use crate::decimal::BadDecimal;
use crate::host::calls::{Answer, CallError, ReadQuery, VenueCall};
use crate::host::legs::LegView;
use crate::strategy::adapter::{Event, Intent};
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

fn by_symbol<W, T>(
    list: &[(String, W)],
    f: impl Fn(&W) -> Result<T, BadDecimal>,
) -> Result<BTreeMap<String, T>, BadDecimal> {
    list.iter().map(|(s, v)| Ok((s.clone(), f(v)?))).collect()
}

pub(crate) fn leg_view(l: &w::LegView) -> Result<LegView, BadDecimal> {
    Ok(LegView {
        id: l.id.clone(),
        venue: l.venue.clone(),
        symbols: l.symbols.clone(),
        books: by_symbol(&l.books, |b| book(b).map(Arc::new))?,
        book_at_ms: l.book_at_ms.iter().cloned().collect(),
        marks: by_symbol(&l.marks, |m| undec(*m))?,
        funding_rates: by_symbol(&l.funding_rates, |r| undec(*r))?,
        positions: l.positions.as_deref().map(positions).transpose()?,
        market: by_symbol(&l.market, market)?,
        reconnected: l.reconnected,
    })
}

/// A shutdown on a leg crosses as a plain shutdown, as the native host lifts it; `Ok(None)` for a nested leg or
/// a request, which the native host refuses with an outcome.
pub(crate) fn leg_intent_to_wire(
    leg: String,
    intent: Intent,
) -> Result<Option<w::Intent>, Uncarried> {
    let intent = match intent {
        Intent::Shutdown(why) => return Ok(Some(w::Intent::Shutdown(why))),
        Intent::Place(o) => w::LegIntent::Place(new_order_to_wire(o)),
        Intent::Cancel(c) => w::LegIntent::Cancel(cancel_to_wire(c)),
        Intent::CancelAll => w::LegIntent::CancelAll,
        Intent::Amend(a) => w::LegIntent::Amend(amend_to_wire(a)),
        Intent::Flatten { symbols } => w::LegIntent::Flatten(symbols),
        Intent::Leg { .. } | Intent::Request { .. } => return Ok(None),
        other => return Err(uncarried(&other)),
    };
    Ok(Some(w::Intent::Leg(w::OnLeg { leg, intent })))
}

pub(crate) fn leg_intent(intent: w::LegIntent) -> Result<Intent, BadDecimal> {
    Ok(match intent {
        w::LegIntent::Place(o) => Intent::Place(new_order(o)?),
        w::LegIntent::Cancel(c) => Intent::Cancel(cancel(c)),
        w::LegIntent::CancelAll => Intent::CancelAll,
        w::LegIntent::Amend(a) => Intent::Amend(amend(a)?),
        w::LegIntent::Flatten(symbols) => Intent::Flatten { symbols },
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

pub(crate) fn leg_event(e: w::LegEvent) -> Result<Event, BadDecimal> {
    let event = match e.event {
        w::LegEventKind::Book => Event::Book,
        w::LegEventKind::Mark(m) => Event::Mark(undec(m)?),
        w::LegEventKind::Fill(t) => Event::Fill(trade(t)?),
        w::LegEventKind::Order(o) => Event::Order(order(&o)?),
    };
    Ok(Event::Leg { leg: e.leg, symbol: e.symbol, event: Box::new(event) })
}
