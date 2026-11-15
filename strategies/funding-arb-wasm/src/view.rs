//! What this plugin builds a [`MarketView`](crate::decide::MarketView) from: the signal and the
//! round-trip cost. **New code of this plugin, not a port.** The built-in's are made by the bot's
//! safety executor, which a plugin does not have; these are deliberately plain, and each says
//! unknown (`None`, which blocks an entry and never forces an exit) rather than guess.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal rates, prices and bps sit far below 7.9e28; divisors are constants or checked non-zero"
)]

use bb_strategy_sdk::{Decimal, FundingSample, OrderBook};
use bb_strategy_support::ioc::two_sided_mid;

/// Settled samples a venue must give before its mean counts.
pub const MIN_SAMPLES: usize = 3;
/// A settlement beyond this per 8h (5%) is bad data and is dropped on its own.
pub const MAX_RATE_8H: Decimal = Decimal::from_parts(5, 0, 0, false, 2);

const BPS: Decimal = Decimal::from_parts(10_000, 0, 0, false, 0);

/// The mean settled rate (per 8h) of `samples`, or `None` with fewer than [`MIN_SAMPLES`] usable.
#[must_use]
pub fn mean_rate(samples: &[FundingSample]) -> Option<Decimal> {
    let usable: Vec<Decimal> =
        samples.iter().map(|s| s.rate_8h).filter(|r| r.abs() <= MAX_RATE_8H).collect();
    if usable.len() < MIN_SAMPLES {
        return None;
    }
    let sum: Decimal = usable.iter().sum();
    sum.checked_div(Decimal::from(usable.len() as u64))
}

/// Bullet's trailing funding minus Hyperliquid's, per 8h: positive means Bullet pays shorts more,
/// so the arb is short Bullet and long Hyperliquid. `None` unless both venues have enough history.
#[must_use]
pub fn signal_8h(bullet: &[FundingSample], hyperliquid: &[FundingSample]) -> Option<Decimal> {
    Some(mean_rate(bullet)? - mean_rate(hyperliquid)?)
}

/// A book's full spread as bps of its mid; `None` for a book that is not two-sided and uncrossed.
fn spread_bps(book: &OrderBook) -> Option<Decimal> {
    let mid = two_sided_mid(book)?;
    let spread = book.best_ask()?.price - book.best_bid()?.price;
    spread.checked_mul(BPS)?.checked_div(mid)
}

/// Enter and exit both legs as a taker, as a fraction of notional: two taker fees on each venue
/// plus each book's spread (half in, half out). `None` when a fee or a book is unknown.
#[must_use]
pub fn round_trip_cost(
    fee_bullet_bps: Option<Decimal>,
    fee_hl_bps: Option<Decimal>,
    book_bullet: &OrderBook,
    book_hl: &OrderBook,
) -> Option<Decimal> {
    let fees = (fee_bullet_bps? + fee_hl_bps?).checked_mul(Decimal::TWO)?;
    let bps = fees + spread_bps(book_bullet)? + spread_bps(book_hl)?;
    bps.checked_div(BPS)
}
