//! bullet-bots' `funding-arb` on the guest SDK: Bullet x Hyperliquid funding carry on one symbol, as
//! a `bb:strategy@0.5.0` component. Short the venue whose trailing funding is higher and long the
//! other, while the carry over `expected_hold_hours` beats the round trip.
//!
//! - [`decide`]: the built-in's `decide`, ported (parity-tested against it).
//! - [`view`]: the signal and the cost, **new code**: the built-in's are the bot's executor's.
//! - [`adapter`]: [`FundingArb`], the execution of `decide`'s target on the two legs, **new code**.
//!   Read its module doc for what it cannot do that the built-in's executor does.
//! - [`config`]: the settings and the descriptor; [`report`]: what it shows of itself.

pub mod adapter;
pub mod config;
pub mod decide;
pub mod report;
pub mod view;

use bb_strategy_sdk::{Describe, Descriptor, FromSettings, LegNeed, StrategyError};

pub use adapter::{FundingArb, HEDGE_LEG};
pub use config::FundingArbSettings;
pub use report::ArbReport;

impl FromSettings for FundingArb {
    fn from_settings(settings: &str) -> Result<Self, StrategyError> {
        let settings: FundingArbSettings = serde_json::from_str(settings)
            .map_err(|e| StrategyError(format!("funding-arb settings: {e}")))?;
        settings.validate()?;
        Ok(Self::new(settings))
    }
}

impl Describe for FundingArb {
    const NAME: &'static str = "funding-arb-wasm";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn describe() -> Descriptor {
        Descriptor {
            summary:
                "Bullet x Hyperliquid funding carry on one symbol: short the venue paying more \
                      funding, long the other, while the carry beats the round trip. Needs a \
                      hedge leg on hyperliquid."
                    .into(),
            settings: config::settings(),
            settings_schema: schemars::schema_for!(FundingArbSettings),
            legs: vec![LegNeed::new(HEDGE_LEG, "hyperliquid")],
        }
    }
}

bb_strategy_sdk::export_strategy!(FundingArb);
