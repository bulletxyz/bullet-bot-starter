//! The ladder's levels as the report shows them, and their prices.

use bb_strategy_sdk::{Decimal, ReportField, Side};
use rust_decimal::RoundingStrategy;
use serde::Serialize;

use crate::LadderSettings;
use crate::level::Level;

/// Where a level is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LevelState {
    /// Sent, no answer yet.
    Pending,
    /// The venue took it (or a cancel of it is asked and not yet answered).
    Resting,
    /// Nothing of it is live: refused for good, or waiting for a book to place it again.
    Refused,
    /// The venue kept cancelling it: held until the bot restarts.
    Held,
}

/// One level of the ladder, as reported (the ledger it comes from is `level.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct Rung {
    pub side: Side,
    /// 1 is nearest the mid.
    pub index: u32,
    pub price: Decimal,
    /// The client id of its last placement.
    pub client_id: Option<String>,
    pub state: LevelState,
    /// Quantity filled of the level in its current cycle (across orders of its remainder).
    pub filled: Decimal,
}

impl Rung {
    /// The level as one report field: `buy 1` is `resting at 75.4, ladder-b1-1` (and what of it
    /// filled, once some has).
    pub(crate) fn field(&self) -> ReportField {
        let side = match self.side {
            Side::Buy => "buy",
            Side::Sell => "sell",
        };
        let state = match self.state {
            LevelState::Pending => "pending",
            LevelState::Resting => "resting",
            LevelState::Refused => "refused",
            LevelState::Held => "held",
        };
        let id = self.client_id.as_deref().unwrap_or("none");
        let text = if self.filled.is_zero() {
            format!("{state} at {}, {id}", self.price)
        } else {
            format!("{state} at {}, {id}, {} filled", self.price, self.filled)
        };
        ReportField::text(format!("{side} {}", self.index), text)
    }
}

/// Digits a price keeps past the mid's own when the venue names no tick.
const EXTRA_DIGITS: u32 = 2;

/// `mid` moved `steps` spacings down (a buy) or up (a sell), on the tick: buys round down, sells
/// up, so a level never comes nearer the mid than its spacing. `None` when it overflows or a buy
/// would be at or below zero.
fn price(
    mid: Decimal,
    side: Side,
    steps: u32,
    spacing_bps: Decimal,
    tick: Option<Decimal>,
) -> Option<Decimal> {
    let offset =
        spacing_bps.checked_mul(Decimal::from(steps))?.checked_div(Decimal::from(10_000))?;
    let factor = match side {
        Side::Buy => Decimal::ONE.checked_sub(offset)?,
        Side::Sell => Decimal::ONE.checked_add(offset)?,
    };
    let raw = mid.checked_mul(factor)?;
    let strategy = match side {
        Side::Buy => RoundingStrategy::ToNegativeInfinity,
        Side::Sell => RoundingStrategy::ToPositiveInfinity,
    };
    let price = match tick.filter(|t| *t > Decimal::ZERO) {
        Some(tick) => {
            raw.checked_div(tick)?.round_dp_with_strategy(0, strategy).checked_mul(tick)?
        }
        None => raw.round_dp_with_strategy(mid.scale().saturating_add(EXTRA_DIGITS), strategy),
    };
    (price > Decimal::ZERO).then_some(price.normalize())
}

/// The ladder around `mid`: buys nearest first, then sells nearest first, none placed yet.
pub(crate) fn levels(
    mid: Decimal,
    settings: &LadderSettings,
    tick: Option<Decimal>,
) -> Option<Vec<Level>> {
    let mut levels = Vec::new();
    for side in [Side::Buy, Side::Sell] {
        for index in 1..=settings.levels {
            let price = price(mid, side, index, settings.spacing_bps, tick)?;
            levels.push(Level::new(side, index, price, settings.order_size));
        }
    }
    Some(levels)
}
