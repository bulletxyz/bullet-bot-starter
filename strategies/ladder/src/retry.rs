//! What the ladder reads from the venue's and the host's words, and from the book. The accounting
//! itself is `level.rs`; this is only classification.
//!
//! - A post-only cross is the venue's own words ([`CROSS_PHRASE`]), else a level whose price
//!   crosses the book the view holds.
//! - A stop ("new orders are stopped", the host's words: a stale book, the daily loss) and the
//!   order rate lift by themselves, so those refusals keep the level for a later book; a limit
//!   (position, notional) is final.
//! - Cancels of several levels within [`SWEEP_MS`] of each other are a sweep (a gap in the book,
//!   the host's stale stop), not the venue refusing a level: they never count toward a level's cap,
//!   nor does a cancel on an unusable book. The order rate limit is the only bound on whole-ladder
//!   sweeps.

use bb_strategy_sdk::OrderBook;

use crate::level::Refusal;
use crate::{RATE_PHRASE, STOPPED_PHRASE};

/// The venue's words for a post-only order refused for crossing (Bullet's, and the paper
/// broker's).
const CROSS_PHRASE: &str = "would cross the book";
/// Cancels of different levels this close together (ms) are one sweep.
pub(crate) const SWEEP_MS: u64 = 2_000;

/// Whether `book` can be quoted against: both sides, and not crossed.
pub(crate) fn usable(book: Option<&OrderBook>) -> bool {
    book.is_some_and(
        |b| matches!((b.best_bid(), b.best_ask()), (Some(bid), Some(ask)) if bid.price < ask.price),
    )
}

/// Why a placement was refused: the reason says when the venue gives one, else a level whose
/// price crosses the book (`crosses_view`) was refused for crossing.
pub(crate) fn refusal_of(reason: Option<&str>, crosses_view: bool) -> Refusal {
    let Some(why) = reason.filter(|r| !r.is_empty()) else {
        return if crosses_view { Refusal::Cross } else { Refusal::Final };
    };
    if why.to_ascii_lowercase().contains(CROSS_PHRASE) {
        Refusal::Cross
    } else if why.contains(STOPPED_PHRASE) {
        Refusal::Stopped
    } else if why.contains(RATE_PHRASE) {
        Refusal::Rate
    } else {
        Refusal::Final
    }
}
