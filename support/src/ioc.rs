//! The price cap on a marketable IOC: the touch crossed by a slippage allowance, never further
//! than a hard distance from a reference (the book's mid, else a recent mark). Pure.

use rust_decimal::Decimal;

use bb_strategy_sdk::{OrderBook, Side};

/// Maximum distance (bps) from mid at which a close-position IOC may trade.
pub const CLOSE_MAX_SLIPPAGE_BPS: u32 = 20;
/// IOC attempts per close-position request.
pub const CLOSE_ATTEMPTS: u32 = 3;
/// No IOC is priced further than this from the reference (bps; the book's mid, else a fresh mark),
/// however far a retry widens it.
pub const MAX_IOC_BPS_FROM_MID: Decimal = Decimal::from_parts(200, 0, 0, false, 0);

const BPS: Decimal = Decimal::from_parts(10_000, 0, 0, false, 0);

/// The mid of a book with both sides, uncrossed; `None` otherwise (empty, one-sided, crossed).
#[must_use]
pub fn two_sided_mid(book: &OrderBook) -> Option<Decimal> {
    let (bid, ask) = (book.best_bid()?.price, book.best_ask()?.price);
    if bid <= Decimal::ZERO || ask <= bid {
        return None;
    }
    bid.checked_add(ask)?.checked_div(Decimal::TWO)
}

/// `a` is within `bps` of `b`.
#[must_use]
pub fn within_bps(a: Decimal, b: Decimal, bps: Decimal) -> bool {
    let Some(gap) = a.checked_sub(b).map(|g| g.abs()) else { return false };
    let Some(allowed) = b.checked_mul(bps).and_then(|v| v.checked_div(BPS)) else {
        return false;
    };
    gap <= allowed
}

/// The limit price of an IOC on `side`: `base` (the touch) crossed by `slip_bps`, and never
/// beyond `max_bps` of `reference` however wide `slip_bps` gets. `None` on overflow or a
/// non-positive result.
#[must_use]
pub fn capped_ioc_price(
    side: Side,
    base: Decimal,
    reference: Decimal,
    slip_bps: Decimal,
    max_bps: Decimal,
) -> Option<Decimal> {
    let slip = slip_bps.checked_div(BPS)?;
    let room = max_bps.checked_div(BPS)?;
    let price = match side {
        Side::Buy => base
            .checked_mul(Decimal::ONE.checked_add(slip)?)?
            .min(reference.checked_mul(Decimal::ONE.checked_add(room)?)?),
        Side::Sell => base
            .checked_mul(Decimal::ONE.checked_sub(slip)?)?
            .max(reference.checked_mul(Decimal::ONE.checked_sub(room)?)?),
    };
    (price > Decimal::ZERO).then_some(price)
}

#[cfg(test)]
#[path = "ioc_tests.rs"]
mod tests;
