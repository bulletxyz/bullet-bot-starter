//! [`AvellanedaStoikovReport`]: what the strategy shows of itself inside the host's snapshot: its
//! tracked quotes and the volatility estimate behind them. Position, P&L and fills are the host's.

use bb_strategy_sdk::{Side, StrategyReport};
use rust_decimal::Decimal;
use serde::Serialize;

use crate::strategy::slots::QuoteSlot;

/// One quote the strategy tracks as resting.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct QuoteReport {
    #[schemars(with = "String")]
    pub side: Side,
    pub level: usize,
    pub price: Decimal,
    pub size: Decimal,
    pub client_id: Option<String>,
    pub order_id: Option<String>,
    /// The bot clock (ms) when the venue acknowledged it.
    pub placed_at_ms: Option<u64>,
}

impl From<&QuoteSlot> for QuoteReport {
    fn from(s: &QuoteSlot) -> Self {
        Self {
            side: s.side,
            level: s.level,
            price: s.price,
            size: s.size,
            client_id: s.client_id.clone(),
            order_id: s.order_id.clone(),
            placed_at_ms: s.placed_at_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct AvellanedaStoikovReport {
    pub slots: Vec<QuoteReport>,
    /// The volatility estimate (stddev of log returns); none until enough samples.
    pub sigma: Option<f64>,
    pub vol_samples: usize,
}

impl StrategyReport for AvellanedaStoikovReport {
    bb_strategy_sdk::typed_report!();

    fn headline(&self) -> String {
        match self.sigma {
            None => format!("warming up: {} price samples so far", self.vol_samples),
            Some(_) => format!("{} quotes resting", self.slots.len()),
        }
    }

    fn schema(&self) -> schemars::Schema {
        schemars::schema_for!(AvellanedaStoikovReport)
    }
}
