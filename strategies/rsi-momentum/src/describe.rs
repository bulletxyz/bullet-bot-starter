//! rsi-momentum as the bot registers it: its name, its settings and its build from them.

use bb_strategy_sdk::{
    Describe, Descriptor, FromSettings, SettingSpec, StrategyError, ValidateConfig,
};

use crate::{RsiMomentum, RsiMomentumConfig};

impl Describe for RsiMomentum {
    const NAME: &'static str = "rsi-momentum";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn describe() -> Descriptor {
        Descriptor {
            summary: "RSI momentum on one Bullet market: long when the RSI crosses up through a \
                      threshold (short on the way down if allowed), out on the exit band or a \
                      stop loss. Directional, unhedged."
                .into(),
            settings: crate::settings().iter().map(SettingSpec::from).collect(),
            settings_schema: schemars::schema_for!(RsiMomentumConfig),
            legs: Vec::new(),
        }
    }
}

impl FromSettings for RsiMomentum {
    fn from_settings(settings: &str) -> Result<Self, StrategyError> {
        let config: RsiMomentumConfig = serde_json::from_str(settings)
            .map_err(|e| StrategyError(format!("rsi-momentum settings: {e}")))?;
        config.validate().map_err(StrategyError)?;
        Ok(Self::new(config))
    }
}

#[cfg(test)]
#[path = "describe_tests.rs"]
mod tests;
