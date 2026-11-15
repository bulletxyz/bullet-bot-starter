//! Helpers the built-in strategies and the plugins share, over the SDK's types: the bot (bb-core
//! re-exports them) and a plugin decide with the same code.
//!
//! - [`client_id`]: the bot's own client ids. A component has no clock, so it seeds from the view's `now_ms`
//!   ([`ClientIdIssuer::own`]); `session_seeded` is seeded from the wall clock natively and from 0 on `wasm32`.
//! - [`ioc`]: the two-sided mid and the IOC price cap, and the close's attempts and slippage cap.
//! - [`close_price`]: what a close is priced off.
//! - [`funding`]: a taker's walk through a book and the trailing median of settled funding.
//! - [`inventory`]: the inventory and realized P&L tracker fills are recorded into.
//! - [`floor_to_step`]: a size rounded down to the lot step.
//! - [`side_word`]: a side as the bot words it; [`clip`]: a report text within the bot's limit.

pub mod client_id;
pub mod close_price;
pub mod funding;
pub mod inventory;
pub mod ioc;

use bb_strategy_sdk::{Decimal, MAX_FIELD_TEXT, MAX_HEADLINE, OrderBook, Side};

pub use client_id::{ClientIdIssuer, is_own};
pub use close_price::{close_reference, touch};
pub use funding::{trailing_median, walk_slip_bps};
pub use inventory::InventoryTracker;

/// The most characters a bot takes in a report's headline or one text field: past it the plugin faults.
pub const MAX_REPORT_TEXT: usize = MAX_FIELD_TEXT;
const _: () = assert!(MAX_HEADLINE == MAX_FIELD_TEXT);

/// `text` cut to at most `max` characters, ending in `…` when cut: a report must never fault for
/// quoting a long refusal.
#[must_use]
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[must_use]
#[expect(
    clippy::arithmetic_side_effects,
    reason = "a size over a positive lot step, far below 7.9e28"
)]
pub fn floor_to_step(qty: Decimal, step: Decimal) -> Decimal {
    if step <= Decimal::ZERO || qty <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    (qty / step).floor() * step
}

/// A side as bb-core's `Display` writes it: `buy` or `sell`.
#[must_use]
pub fn side_word(side: Side) -> &'static str {
    match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

/// Kept so plugins that import it still build; the SDK's book has it now.
#[deprecated(note = "use the inherent `OrderBook::would_cross`")]
pub trait BookExt {
    fn would_cross(&self, side: Side, price: Decimal) -> bool;
}

#[expect(deprecated, reason = "the deprecated shim's own impl")]
impl BookExt for OrderBook {
    fn would_cross(&self, side: Side, price: Decimal) -> bool {
        OrderBook::would_cross(self, side, price)
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
