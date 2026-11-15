//! [`GridReport`]: what the grid shows of itself inside the host's snapshot: its plan (range, step,
//! size, stops, worst case), its levels, whether it paused or stopped, and the P&L of its whole
//! position. Orders and the position itself are the host's.

use bb_strategy_sdk::StrategyReport;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::levels::GridLevel;

/// The grid's own state, serialized as it is (MCP and the web UI show it as-is). The plan's fields
/// are `None` until the start resolved it.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct GridReport {
    /// `neutral`, `long` or `short`.
    pub direction: Option<String>,
    /// The range preset: `tight`, `normal` or `wide`.
    pub range: String,
    pub lower_price: Option<Decimal>,
    pub upper_price: Option<Decimal>,
    /// The price the levels were laid out around.
    pub anchor: Option<Decimal>,
    pub spacing: Option<Decimal>,
    pub order_size: Option<Decimal>,
    /// The hard stop below the range, when the grid holds a long side.
    pub stop_below: Option<Decimal>,
    /// The hard stop above the range, when the grid holds a short side.
    pub stop_above: Option<Decimal>,
    /// What the position costs with every losing-side level filled (USD).
    pub worst_case_usd: Option<Decimal>,
    /// The dollars it was sized from.
    pub amount_usd: Option<Decimal>,
    pub active_levels: usize,
    pub total_levels: usize,
    /// Its opening orders are held back: by the trend filter, or out of range on the profitable
    /// side.
    pub paused: bool,
    /// Why it stopped trading (the hard stop or the daily loss stop), when it did.
    pub stopped: Option<String>,
    pub pnl: GridPnl,
    pub levels: Vec<GridLevel>,
    /// How the trend filter's warm-up on past candles went, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<String>,
}

/// The whole position's P&L, as the host recorded it, in USD.
#[derive(Debug, Clone, Default, Serialize, schemars::JsonSchema)]
pub struct GridPnl {
    pub realized: Decimal,
    pub unrealized: Decimal,
    pub total: Decimal,
}

impl StrategyReport for GridReport {
    bb_strategy_sdk::typed_report!();
    fn headline(&self) -> String {
        if let Some(why) = &self.stopped {
            return format!("stopped: {why}");
        }
        let (Some(lower), Some(upper)) = (self.lower_price, self.upper_price) else {
            return "not started".into();
        };
        let paused = if self.paused { ", its opening orders paused" } else { "" };
        format!(
            "{} of {} levels resting between {} and {}{paused}",
            self.active_levels,
            self.total_levels,
            lower.normalize(),
            upper.normalize()
        )
    }

    fn schema(&self) -> schemars::Schema {
        schemars::schema_for!(GridReport)
    }

    fn briefing_lines(&self) -> Vec<String> {
        self.history.iter().cloned().collect()
    }
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
