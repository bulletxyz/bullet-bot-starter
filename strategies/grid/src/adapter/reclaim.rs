//! A start that takes over the position a kept shutdown left (`instance.on_shutdown = keep`, money
//! safety): the host seeded it from the venue and cancelled every order on the market, as at any
//! live start. The levels are placed again as at any start, but held to the plan's worst case on
//! each side with the kept position counted (a level the run before filled is not laid again until
//! the position comes back down), and no starting IOC goes out over a position already held.

use bb_strategy_sdk::Side;
use rust_decimal::Decimal;

use super::GridStrategy;
use crate::derive::Direction;

impl GridStrategy {
    /// The plan's worst case on each side, long and short, in coins: a neutral grid's buys below
    /// and sells above; a long grid all of it long (its starting IOC bought the far side), a short
    /// one all of it short.
    pub(super) fn worst_by_side(&self, order_size: Decimal) -> Option<(Decimal, Decimal)> {
        let plan = self.plan.as_ref()?;
        let count = |side: Side| {
            let n = self.state.levels.iter().filter(|l| l.side == Some(side)).count();
            Decimal::from(u64::try_from(n).unwrap_or(u64::MAX)) * order_size
        };
        Some(match plan.direction {
            Direction::Neutral => (count(Side::Buy), count(Side::Sell)),
            Direction::Long => (plan.worst_coins, Decimal::ZERO),
            Direction::Short => (Decimal::ZERO, plan.worst_coins),
        })
    }
}
