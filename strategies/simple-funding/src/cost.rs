//! Bullet-only round-trip cost: two taker fees plus the book walk in and out.

use bb_strategy_sdk::OrderBook;
use bb_strategy_support::walk_slip_bps;
use rust_decimal::Decimal;

const BPS: i64 = 10_000;

/// `(price, qty)` best first, empty levels dropped: asks for a buy, bids for a sell.
pub fn levels(book: &OrderBook, buy: bool) -> Vec<(Decimal, Decimal)> {
    let side: Vec<(Decimal, Decimal)> = if buy {
        book.asks.iter().map(|(p, q)| (*p, *q)).collect()
    } else {
        book.bids.iter().rev().map(|(p, q)| (*p, *q)).collect()
    };
    side.into_iter().filter(|(_, q)| *q > Decimal::ZERO).collect()
}

/// Enter and exit as a taker, as a fraction of notional: `(2 × fee + slip_in + slip_out)
/// / 10_000`, the slips being each walk's average fill past the mid (half the spread
/// plus depth impact). `None` if any input is unknown or the sum overflows.
pub fn round_trip_cost(
    fee_bps: Option<Decimal>,
    slip_in_bps: Option<Decimal>,
    slip_out_bps: Option<Decimal>,
) -> Option<Decimal> {
    let bps = fee_bps?
        .checked_mul(Decimal::TWO)?
        .checked_add(slip_in_bps?)?
        .checked_add(slip_out_bps?)?;
    bps.checked_div(Decimal::from(BPS))
}

/// Whether `qty` base units fill at or inside `limit` (a buy takes asks up to it, a sell takes
/// bids down to it): the book walk an IOC capped at `limit` can actually complete. A book
/// whose depth lies beyond the cap, or a book too thin, is `false`.
#[must_use]
pub fn fills_within(book: &OrderBook, buy: bool, qty: Decimal, limit: Decimal) -> bool {
    let mut left = qty;
    for (price, size) in levels(book, buy) {
        let inside = if buy { price <= limit } else { price >= limit };
        if !inside {
            return false;
        }
        let Some(rest) = left.checked_sub(size) else { return false };
        left = rest;
        if left <= Decimal::ZERO {
            return true;
        }
    }
    false
}

/// Whether `notional` USD can be bought and sold inside `slippage_bps` of the touch.
#[must_use]
pub fn fillable_within(book: &OrderBook, notional: Decimal, slippage_bps: Decimal) -> bool {
    let (Some(ask), Some(bid), Some(mid)) =
        (book.best_ask(), book.best_bid(), book.midpoint().filter(|m| *m > Decimal::ZERO))
    else {
        return false;
    };
    let Some(qty) = notional.checked_div(mid) else { return false };
    let slip = slippage_bps.checked_div(Decimal::from(BPS)).unwrap_or(Decimal::ZERO);
    let up = ask.price.checked_mul(Decimal::ONE.saturating_add(slip));
    let down = bid.price.checked_mul(Decimal::ONE.saturating_sub(slip));
    matches!((up, down), (Some(up), Some(down))
        if fills_within(book, true, qty, up) && fills_within(book, false, qty, down))
}

/// USD resting on one side of the book (`buy`: the asks).
fn side_usd(book: &OrderBook, buy: bool) -> Option<Decimal> {
    levels(book, buy)
        .iter()
        .try_fold(Decimal::ZERO, |sum, (p, q)| sum.checked_add(p.checked_mul(*q)?))
}

/// What a book says about trading `notional` USD in and out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookCost {
    /// Round trip as a fraction of notional. `None` = unknown fee or a book too thin to fill.
    pub round_trip: Option<Decimal>,
    /// The thinner side's resting USD. `None` = a side is empty.
    pub depth_usd: Option<Decimal>,
}

/// Cost and depth of `book` for `notional` USD. Walking asks then bids costs the same sum
/// whichever side opens, so one number serves a long and a short.
pub fn assess(book: &OrderBook, fee_bps: Option<Decimal>, notional: Decimal) -> BookCost {
    let depth_usd = match (side_usd(book, true), side_usd(book, false)) {
        (Some(a), Some(b)) if a > Decimal::ZERO && b > Decimal::ZERO => Some(a.min(b)),
        _ => None,
    };
    let round_trip = book.midpoint().filter(|m| *m > Decimal::ZERO).and_then(|mid| {
        let qty = notional.checked_div(mid)?;
        let buy = walk_slip_bps(&levels(book, true), true, mid, qty);
        let sell = walk_slip_bps(&levels(book, false), false, mid, qty);
        round_trip_cost(fee_bps, buy, sell)
    });
    BookCost { round_trip, depth_usd }
}

#[cfg(test)]
#[path = "cost_tests.rs"]
mod tests;
