//! Shared inventory / realized-PnL tracker. Strategies call `record_fill` from
//! their `EventHandler<Trade>` impl; the tracker maintains net position,
//! weighted-average entry price, and realized `PnL` using the standard
//! open-closed split.
//!
//! Units: base-asset quantity, quote-asset price, quote-asset realized `PnL`.
//! Buys increase position; sells decrease. Crossing zero splits a fill into
//! a closing-portion (realized against prior avg entry) and an opening-portion
//! (becomes the new avg entry).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use bb_strategy_sdk::{Position, Side};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InventoryTracker {
    pub net_position: Decimal,
    /// Volume-weighted average entry price of the open portion. Zero when flat.
    pub avg_entry_price: Decimal,
    pub realized_pnl: Decimal,
    pub total_fills: u64,
}

impl InventoryTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a fill. Returns the realized `PnL` contribution from this fill
    /// (nonzero only when the fill closes or partially closes an existing
    /// position), or `None`, the tracker unchanged, when the position, its
    /// entry price, the realized `PnL` or the fill's notional would overflow a `Decimal`.
    pub fn record_fill(
        &mut self,
        side: Side,
        price: Decimal,
        quantity: Decimal,
    ) -> Option<Decimal> {
        let fills = self.total_fills.checked_add(1)?;
        // A fill whose notional is no `Decimal` is one no reader of it could price.
        price.checked_mul(quantity)?;
        let signed_qty = match side {
            Side::Buy => quantity,
            Side::Sell => Decimal::ZERO.checked_sub(quantity)?,
        };

        // Flat → opening a new position.
        if self.net_position.is_zero() {
            (self.total_fills, self.net_position, self.avg_entry_price) =
                (fills, signed_qty, price);
            return Some(Decimal::ZERO);
        }

        let net = self.net_position.checked_add(signed_qty)?;
        let same_direction = self.net_position.is_sign_positive() == signed_qty.is_sign_positive();
        if same_direction {
            // Adding to an existing position — weighted-average the entry price.
            let old_abs = self.net_position.abs();
            let add_abs = signed_qty.abs();
            let cost = self
                .avg_entry_price
                .checked_mul(old_abs)?
                .checked_add(price.checked_mul(add_abs)?)?;
            let avg = cost.checked_div(old_abs.checked_add(add_abs)?)?;
            (self.total_fills, self.net_position, self.avg_entry_price) = (fills, net, avg);
            return Some(Decimal::ZERO);
        }

        // Opposite direction — closing (possibly flipping).
        let close_abs = self.net_position.abs().min(signed_qty.abs());
        let pnl_per_unit = if self.net_position.is_sign_positive() {
            price.checked_sub(self.avg_entry_price)? // closing long: sell high
        } else {
            self.avg_entry_price.checked_sub(price)? // closing short: buy low
        };
        let realized = pnl_per_unit.checked_mul(close_abs)?;
        let realized_pnl = self.realized_pnl.checked_add(realized)?;
        let remaining = signed_qty.abs().checked_sub(close_abs)?;
        let avg = if net.is_zero() {
            Decimal::ZERO
        } else if remaining > Decimal::ZERO {
            // Flipped — the leftover opens a new position at the fill price.
            price
        } else {
            self.avg_entry_price
        };
        (self.total_fills, self.net_position, self.avg_entry_price) = (fills, net, avg);
        self.realized_pnl = realized_pnl;
        Some(realized)
    }

    pub fn is_flat(&self) -> bool {
        self.net_position.is_zero()
    }

    /// Seed the tracker from a venue-reported position, typically at strategy
    /// startup after reconnecting/restarting with existing exposure.
    #[expect(clippy::arithmetic_side_effects, reason = "Decimal negation only flips the sign bit")]
    pub fn seed_from_position(&mut self, position: &Position) {
        self.net_position = match position.side {
            Some(Side::Buy) => position.size,
            Some(Side::Sell) => -position.size,
            None => Decimal::ZERO,
        };
        self.avg_entry_price =
            if self.net_position.is_zero() { Decimal::ZERO } else { position.entry_price };
    }
}

#[cfg(test)]
#[path = "inventory_tests.rs"]
mod tests;
