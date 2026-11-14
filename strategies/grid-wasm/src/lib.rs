//! bullet-bots' `grid` on the guest SDK: a futures grid on one Bullet market, as a
//! `bb:strategy@0.4.0` component. Resting buys below the price and sells above across the range
//! the settings give, each fill re-armed one level over; a hard stop past the range flattens and
//! stops it.
//!
//! - [`derive`], [`plan`], [`grid`], [`ema`]: the built-in's files, unchanged but for imports.
//! - [`adapter`]: [`GridStrategy`], the built-in's adapter without what the world cannot carry (its
//!   module doc lists it); [`pull`]: its cancel-all retry, without the alert.
//! - [`config`]: the settings and the descriptor; [`report`]: what it shows of itself.

pub mod adapter;
pub mod config;
pub mod derive;
pub mod ema;
pub mod grid;
pub mod plan;
pub mod pull;
pub mod report;

use bb_strategy_sdk::{Describe, Descriptor, FromSettings, StrategyError};

pub use adapter::GridStrategy;
pub use config::GridConfig;
pub use report::GridReport;

impl FromSettings for GridStrategy {
    fn from_settings(settings: &str) -> Result<Self, StrategyError> {
        let config: GridConfig = serde_json::from_str(settings)
            .map_err(|e| StrategyError(format!("grid settings: {e}")))?;
        config.check()?;
        Ok(Self::new(config))
    }
}

impl Describe for GridStrategy {
    const NAME: &'static str = "grid-wasm";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn describe() -> Descriptor {
        Descriptor {
            summary: "Futures grid on one market: resting buys below the price and sells above \
                      across the range you set, each fill re-armed one level over; a hard stop \
                      past the range flattens and stops it."
                .into(),
            settings: config::settings(),
            settings_schema: schemars::schema_for!(GridConfig),
            legs: Vec::new(),
        }
    }
}

bb_strategy_sdk::export_strategy!(GridStrategy);
