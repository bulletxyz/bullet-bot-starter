//! A dollar-neutral funding carry basket on Bullet markets, as a `bb:strategy@0.5.0` component.
//! Short the markets whose settled funding pays shorts, long the ones that pay longs, equal USD
//! per leg, entered as pairs and kept dollar-neutral within `max_net_pct`. Dollar-neutral is not
//! price-neutral: the legs are different assets and move differently (the README says how much
//! that can cost). Paper first.
//!
//! - [`decide`], [`plan`], [`cost`], [`signal`], [`limits`]: the strategy's decision core, pure.
//! - [`actor`], [`adapter`], [`io`]: the rules that read the venue and trade, on the world's
//!   requests (the module docs of [`actor`] list what this cannot be).
//! - [`config`]: the settings and the descriptor; [`report`]: what it shows of itself.

pub mod actor;
pub mod adapter;
pub mod config;
pub mod cost;
pub mod decide;
pub mod io;
pub mod limits;
pub mod plan;
pub mod report;
pub mod signal;

use bb_strategy_sdk::{Describe, Descriptor, FromSettings, LegNeed, StrategyError};

pub use adapter::SimpleFunding;
pub use config::SimpleFundingSettings;
pub use report::Status;

impl FromSettings for SimpleFunding {
    fn from_settings(settings: &str) -> Result<Self, StrategyError> {
        let settings: SimpleFundingSettings = serde_json::from_str(settings)
            .map_err(|e| StrategyError(format!("simple-funding settings: {e}")))?;
        settings.validate()?;
        Ok(Self::new(settings))
    }
}

impl Describe for SimpleFunding {
    const NAME: &'static str = "simple-funding-wasm";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn describe() -> Descriptor {
        Descriptor {
            summary: "Dollar-neutral funding carry across many Bullet markets: short the top \
                      positive funding, long the most negative, equal USD per leg, entered as \
                      pairs. Dollar-neutral is not price-neutral. Needs the leg `universe` on \
                      bullet with the markets to trade."
                .into(),
            settings: config::settings(),
            settings_schema: schemars::schema_for!(SimpleFundingSettings),
            legs: vec![LegNeed::new(actor::UNIVERSE, actor::VENUE)],
        }
    }
}

bb_strategy_sdk::export_strategy!(SimpleFunding);

#[cfg(test)]
mod tests;
