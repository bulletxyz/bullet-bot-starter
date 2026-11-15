//! bullet-bots' `avellaneda-stoikov` on the guest SDK: a market maker whose quotes come from the
//! Avellaneda-Stoikov closed form (reservation price skewed by inventory, a spread from the
//! volatility and the risk aversion), laddered over `order_levels` and kept in place by amending
//! only what drifted. A `bb:strategy@0.5.0` component.
//!
//! - [`model`], [`volatility`]: the built-in's math, the volatility estimator on the bot's
//!   milliseconds instead of the OS clock.
//! - [`adapter`]: [`AvellanedaStoikov`], the built-in's actor as intents (its module doc lists what
//!   the world cannot carry).
//! - [`config`]: the settings and the descriptor; [`report`]: what it shows of itself.

pub mod adapter;
pub mod config;
pub mod model;
pub mod report;
pub mod volatility;

use bb_strategy_sdk::{Describe, Descriptor, FromSettings, LegNeed, StrategyError};

pub use adapter::AvellanedaStoikov;
pub use config::AvellanedaStoikovSettings;
pub use report::AsReport;

/// The leg the reference price comes from (`legs.reference.symbols` on the bot; Binance).
pub const REFERENCE_LEG: &str = "reference";

impl FromSettings for AvellanedaStoikov {
    fn from_settings(settings: &str) -> Result<Self, StrategyError> {
        let settings: AvellanedaStoikovSettings = serde_json::from_str(settings)
            .map_err(|e| StrategyError(format!("avellaneda-stoikov settings: {e}")))?;
        settings.validate()?;
        Ok(Self::new(settings))
    }
}

impl Describe for AvellanedaStoikov {
    const NAME: &'static str = "avellaneda-stoikov-wasm";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn describe() -> Descriptor {
        Descriptor {
            summary: "Avellaneda-Stoikov market maker: a spread and an inventory skew from a \
                      volatility and risk-aversion model, in levels on both sides, optionally \
                      priced off Binance."
                .into(),
            settings: config::settings(),
            settings_schema: schemars::schema_for!(AvellanedaStoikovSettings),
            legs: vec![LegNeed::new(REFERENCE_LEG, "binance")],
        }
    }
}

bb_strategy_sdk::export_strategy!(AvellanedaStoikov);
