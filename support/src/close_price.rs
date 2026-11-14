//! What a close is priced off. A book of any age is not a price: after a feed stall it can sit far
//! from where the market is. A strategy asks [`close_reference`] and gets the book only while it is
//! fresh and agrees with a fresh mark; otherwise the mark. The book's age is the strategy's to
//! keep (when it last saw [`Event::Book`](bb_strategy_sdk::Event::Book)), on the bot clock.

use crate::ioc::{two_sided_mid, within_bps};
use bb_strategy_sdk::{OrderBook, Side};
use rust_decimal::Decimal;

/// The price `side` crosses at: the best ask for a buy, the best bid for a sell.
#[must_use]
pub fn touch(book: &OrderBook, side: Side) -> Option<Decimal> {
    match side {
        Side::Buy => book.best_ask().map(|l| l.price),
        Side::Sell => book.best_bid().map(|l| l.price),
    }
    .filter(|p| *p > Decimal::ZERO)
}

/// `(reference, base)` to price an IOC of `side` from, for a slippage capped around `reference`
/// (see `capped_ioc_price`): the book's mid and touch when the book is fresh (`book_age_ms` at most
/// `stale_ms`), two-sided, and within `max_bps` of `fresh_mark` (any fresh book when there is no
/// fresh mark to compare); else the fresh mark itself; else `None` (the caller's own fallback, e.g.
/// the entry price).
#[must_use]
pub fn close_reference(
    book: Option<&OrderBook>,
    book_age_ms: Option<u64>,
    side: Side,
    fresh_mark: Option<Decimal>,
    stale_ms: u64,
    max_bps: Decimal,
) -> Option<(Decimal, Decimal)> {
    let fresh = book_age_ms.is_some_and(|age| age <= stale_ms);
    let book = book.filter(|_| fresh);
    match (book.and_then(two_sided_mid), fresh_mark) {
        (Some(mid), Some(mark)) if !within_bps(mid, mark, max_bps) => Some((mark, mark)),
        (Some(mid), _) => Some((mid, book.and_then(|b| touch(b, side)).unwrap_or(mid))),
        (None, Some(mark)) => Some((mark, mark)),
        (None, None) => None,
    }
}

#[cfg(test)]
#[path = "close_price_tests.rs"]
mod tests;
