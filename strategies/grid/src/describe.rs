//! The grid as the bot registers it: its name, its settings and its build from them.

use bb_strategy_sdk::{
    Describe, Descriptor, FromSettings, SettingSpec, StrategyError, ValidateConfig,
};

use crate::{GridConfig, GridStrategy};

impl Describe for GridStrategy {
    const NAME: &'static str = "grid";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn describe() -> Descriptor {
        Descriptor {
            summary: "Futures grid on one market: resting buys below the price and sells above \
                      across a range derived from its volatility, each fill re-armed one level \
                      over; a hard stop past the range flattens and stops it."
                .into(),
            settings: crate::config::settings().iter().map(SettingSpec::from).collect(),
            settings_schema: schemars::schema_for!(GridConfig),
            legs: Vec::new(),
        }
    }
}

impl FromSettings for GridStrategy {
    fn from_settings(settings: &str) -> Result<Self, StrategyError> {
        let config: GridConfig = serde_json::from_str(settings)
            .map_err(|e| StrategyError(format!("grid settings: {e}")))?;
        config.validate().map_err(StrategyError)?;
        Ok(Self::new(config))
    }
}

#[cfg(test)]
#[path = "describe_tests.rs"]
mod tests;
