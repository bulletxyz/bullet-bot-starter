//! bullet-bots' `rsi-momentum` on the guest SDK: RSI momentum on one Bullet market, as a
//! `bb:strategy@0.4.0` component. Directional and unhedged: long when the RSI of closed candles
//! crosses up through `long_above` (short through `short_below` only with `allow_short`), out
//! past `exit_band` or at `stop_loss_pct` against the entry.
//!
//! - [`candles`], [`rsi`], [`decide`], [`limits`]: the built-in's files, unchanged but for imports.
//! - [`adapter`]: [`RsiMomentum`], the built-in's adapter without what the world cannot carry
//!   (its module doc lists it).
//! - [`config`]: the settings and the descriptor; [`report`]: what it shows of itself.

pub mod adapter;
pub mod candles;
pub mod config;
pub mod decide;
pub mod limits;
pub mod report;
pub mod rsi;

use bb_strategy_sdk::{Describe, Descriptor, FromSettings, StrategyError};

pub use adapter::RsiMomentum;
pub use config::RsiMomentumSettings;
pub use report::RsiReport;

impl FromSettings for RsiMomentum {
    fn from_settings(settings: &str) -> Result<Self, StrategyError> {
        let settings: RsiMomentumSettings = serde_json::from_str(settings)
            .map_err(|e| StrategyError(format!("rsi-momentum settings: {e}")))?;
        settings.validate()?;
        Ok(Self::new(settings))
    }
}

impl Describe for RsiMomentum {
    const NAME: &'static str = "rsi-momentum-wasm";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn describe() -> Descriptor {
        Descriptor {
            summary: "RSI momentum on one Bullet market: long when the RSI crosses up through a \
                      threshold (short on the way down if allowed), out on the exit band or a stop \
                      loss. Directional, unhedged."
                .into(),
            settings: config::settings(),
            settings_schema: schemars::schema_for!(RsiMomentumSettings),
            legs: Vec::new(),
        }
    }
}

bb_strategy_sdk::export_strategy!(RsiMomentum);
