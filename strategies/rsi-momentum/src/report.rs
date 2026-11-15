//! [`RsiReport`]: what the strategy shows of itself inside the host's snapshot. Position, P&L,
//! orders and the daily loss stop are the host's.

use bb_strategy_sdk::StrategyReport;
use rust_decimal::Decimal;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct RsiReport {
    pub symbol: String,
    pub interval: String,
    pub rsi: Option<Decimal>,
    /// Closed candles still needed before the RSI reads (and then one more for a cross).
    pub closes_to_warm: u32,
    pub mark: Option<Decimal>,
    pub cooldown_candles_left: u32,
    /// A close is under way: its attempts so far.
    pub close_attempts: Option<u32>,
    /// A close has failed past its attempts and is still being retried every tick.
    pub close_failing: bool,
    /// How the warm-up went: on past candles (how many, from where), or from live data and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<String>,
}

impl StrategyReport for RsiReport {
    bb_strategy_sdk::typed_report!();
    fn headline(&self) -> String {
        if self.close_failing {
            return format!("{}: a close keeps failing, retrying every tick", self.symbol);
        }
        match (self.rsi, self.closes_to_warm) {
            (Some(rsi), _) => format!("{} RSI {rsi} on {} candles", self.symbol, self.interval),
            (None, n) => format!(
                "{}: warming up, {n} more {} candles for the first RSI",
                self.symbol, self.interval
            ),
        }
    }

    fn schema(&self) -> schemars::Schema {
        schemars::schema_for!(RsiReport)
    }

    /// The warm-up's note while the RSI is still warming up (a load that failed, or was dropped).
    fn briefing_lines(&self) -> Vec<String> {
        self.history.iter().filter(|_| self.closes_to_warm > 0).cloned().collect()
    }
}
