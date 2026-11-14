//! Helpers the bundled strategies share, over the SDK's types. Each module is a copy of the
//! bullet-bots code it names, kept the same so a port decides as the built-in does.
//!
//! - [`client_id`]: bb-core's `helpers::client_id` (the bot's own client ids), without the
//!   wall-clock seed: a component has no clock, so it seeds from the view's `now_ms`.
//! - [`ioc`]: bb-core's `helpers::ioc` (the two-sided mid and the IOC price cap).
//! - [`close_price`]: bb-strategy's `close_price` (what a close is priced off).
//! - [`BookExt`]: bb-core's `OrderBook::would_cross`, which the SDK's book does not have.
//! - [`side_word`]: bb-core's `Display` for `Side`; [`clip`]: a report text within the bot's limit.

pub mod client_id;
pub mod close_price;
pub mod ioc;

use bb_strategy_sdk::{Decimal, OrderBook, Side};

pub use client_id::{ClientIdIssuer, is_own};
pub use close_price::{close_reference, touch};

/// The most characters a bot takes in a report's headline or one text field: past it the plugin
/// faults (bb-strategy-wasm's `MAX_HEADLINE`, `MAX_FIELD_TEXT`).
pub const MAX_REPORT_TEXT: usize = 200;

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

/// A side as bb-core's `Display` writes it: `buy` or `sell`.
#[must_use]
pub fn side_word(side: Side) -> &'static str {
    match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

/// What bb-core's `OrderBook` has that the SDK's does not.
pub trait BookExt {
    /// Whether an order at `price` would cross: buys at or above the best ask, sells at or below
    /// the best bid. An empty side is no information, so it does not cross.
    fn would_cross(&self, side: Side, price: Decimal) -> bool;
}

impl BookExt for OrderBook {
    fn would_cross(&self, side: Side, price: Decimal) -> bool {
        match side {
            Side::Buy => self.best_ask().is_some_and(|ask| price >= ask.price),
            Side::Sell => self.best_bid().is_some_and(|bid| price <= bid.price),
        }
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
