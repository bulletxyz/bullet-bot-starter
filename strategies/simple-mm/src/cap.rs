//! The market maker's own cap on a quote (money safety): the size it may send at the position it
//! holds. It is the host's order gate's rule, applied before the quote is sent, so the market
//! maker never asks for an order the gate would refuse and drops the side that would grow the
//! position past its cap rather than send it.

#![expect(clippy::arithmetic_side_effects, reason = "Decimal sizes and prices far below 7.9e28")]

use bb_strategy_sdk::{LimitsInForce, Side};
use bb_strategy_support::floor_to_step;
use rust_decimal::Decimal;

use crate::settings::MarketMakerSettings;

/// The caps a quote is held to: the section's, tightened by the host's limits when it shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub max_position: Decimal,
    pub max_notional_usd: Option<Decimal>,
    /// The venue's size step a clamped size is rounded down to.
    pub step: Option<Decimal>,
}

impl Caps {
    #[must_use]
    pub fn new(
        settings: &MarketMakerSettings,
        limits: Option<&LimitsInForce>,
        step: Option<Decimal>,
    ) -> Self {
        let lower = |a: Option<Decimal>, b: Option<Decimal>| match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let max_position =
            limits.map_or(settings.max_position, |l| settings.max_position.min(l.max_position));
        let max_notional_usd =
            lower(settings.max_notional_usd, limits.and_then(|l| l.max_notional_usd));
        Self { max_position, max_notional_usd, step: step.filter(|s| *s > Decimal::ZERO) }
    }

    /// The size of a `qty` quote on `side` at `price`, with the net position at `net`: at most
    /// the position cap (one order may be as big as the cap, never past it), rounded down to the
    /// step when clamped. `None` (the side is not quoted): it would grow the position past the
    /// position or notional cap, or its size is zero.
    #[must_use]
    pub fn allowed(
        &self,
        side: Side,
        qty: Decimal,
        price: Decimal,
        net: Decimal,
    ) -> Option<Decimal> {
        let clamped = qty.min(self.max_position);
        if clamped <= Decimal::ZERO {
            return None;
        }
        // The position after a fill, in the order's direction: positive grows this side.
        let exposure = match side {
            Side::Buy => net.checked_add(clamped)?,
            Side::Sell => clamped.checked_sub(net)?,
        };
        if exposure > self.max_position {
            return None;
        }
        if let Some(cap) = self.max_notional_usd
            && (price <= Decimal::ZERO || exposure.checked_mul(price).is_none_or(|n| n > cap))
        {
            return None;
        }
        if clamped == qty {
            return Some(qty);
        }
        let lots = self.step.map_or(clamped, |s| floor_to_step(clamped, s));
        (lots > Decimal::ZERO).then_some(lots)
    }

    /// `price` and `qty` pass unchanged with `old` (a resting quote on `side`) counted as filled:
    /// a reprice may then go out as one amend, since the venue skips the cancel of an order that
    /// no longer rests and the replacement still places.
    #[must_use]
    pub fn passes_with_filled(
        &self,
        side: Side,
        qty: Decimal,
        price: Decimal,
        net: Decimal,
        old: Decimal,
    ) -> bool {
        let filled = match side {
            Side::Buy => net + old,
            Side::Sell => net - old,
        };
        self.allowed(side, qty, price, filled) == Some(qty)
    }
}

#[cfg(test)]
#[path = "cap_tests.rs"]
mod tests;
