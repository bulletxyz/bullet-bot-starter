//! [`MarketMakerReport`]: what the market maker shows of itself in the host's snapshot. Its JSON is
//! what MCP (`bot.report`) and the web UI read of a market maker. Position, P&L, orders and the
//! stops are the host's.

use bb_strategy_sdk::{PaperInputs, StrategyReport};
use rust_decimal::Decimal;
use serde::Serialize;

use crate::params::{ExtraParam, FairValue, MmParams};
use crate::pricing::Fallback;

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct MarketMakerReport {
    pub params: MmParams,
    /// The strategy's own params (name, bounds, help); their values are in `params.extra`.
    pub extra_params: Vec<ExtraParam>,
    /// The maker fee the section names, in basis points (0 when none is).
    pub maker_fee_bps: Decimal,
    /// Seconds this run spent with a quote up and no stop in force.
    pub paper_quoting_secs: u64,
    /// Where the fair value came from: `binance` (the Binance mid plus the basis) or `bullet`.
    /// `None` until there is a Bullet price to quote around.
    pub fair_source: Option<FairValue>,
    /// Why the Bullet mid is used although `fair_value = "binance"`.
    pub fair_fallback: Option<Fallback>,
    /// The fair value at the last event (what the next refresh centres on).
    pub fair_value: Option<Decimal>,
    /// Average Bullet mid minus Binance mid, in bps of the Bullet mid.
    pub basis_bps: Option<Decimal>,
}

impl MarketMakerReport {
    /// What the quotes are centred on, in the venues' own names.
    #[must_use]
    pub fn fair_sentence(&self) -> String {
        let traded = FairValue::TRADED.venue();
        let chosen = self.params.fair_value;
        match (self.fair_source, self.fair_fallback) {
            (None, _) if chosen.is_external() => {
                let other = chosen.venue();
                format!(
                    "Quotes will move to the {other} mid plus the average {traded}-{other} gap after \
                     about a minute of averaging that gap, and stay on the {traded} mid until then. "
                )
            }
            (None, _) => {
                format!("Quotes will be centred on the {traded} mid once it has a price. ")
            }
            (Some(source), _) if source.is_external() => {
                let other = source.venue();
                let gap = self.basis_bps.map(|b| format!(" ({b:+} bps)")).unwrap_or_default();
                format!(
                    "Quotes are centred on the {other} mid plus the average {traded}-{other} gap{gap}. "
                )
            }
            (Some(_), Some(why)) => {
                format!("Quotes are centred on the {traded} mid for now: {why}. ")
            }
            (Some(_), None) => format!("Quotes are centred on the {traded} mid. "),
        }
    }
}

impl StrategyReport for MarketMakerReport {
    bb_strategy_sdk::typed_report!();
    fn headline(&self) -> String {
        self.fair_sentence().trim_end().to_string()
    }

    fn schema(&self) -> schemars::Schema {
        schemars::schema_for!(MarketMakerReport)
    }

    fn paper_inputs(&self) -> Option<PaperInputs> {
        Some(PaperInputs {
            order_size: self.params.order_size,
            quoting_secs: self.paper_quoting_secs,
            maker_fee_bps: self.maker_fee_bps,
        })
    }

    fn briefing_lines(&self) -> Vec<String> {
        vec![self.fair_sentence()]
    }

    fn reference_price(&self) -> Option<Decimal> {
        self.fair_value
    }
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
