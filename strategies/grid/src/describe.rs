//! The grid as the bot registers it: its name, its settings and its build from them.

use std::collections::BTreeMap;

use bb_strategy_sdk::profile::{Amounts, PresetMarket, money, text};
use bb_strategy_sdk::{
    About, Describe, Descriptor, ExchangeNeed, Featured, FromSettings, Preset, Profile, Risk,
    RiskLevel, SettingSpec, StrategyError, ValidateConfig,
};
use rust_decimal::Decimal;

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

    fn profile() -> Option<Profile> {
        Some(Profile {
            about: About {
                title: "Grid",
                tagline: "Buys low, sells high in a range",
                blurb: "Rests buy orders below the price and sell orders above it across a range \
                        set from how much the market moves, and sells each buy one level higher.",
                risk: Risk {
                    level: RiskLevel::MediumHigh,
                    text: "Earns when the price swings back and forth inside a range; loses when \
                           it trends through it, which the stop below the range limits.",
                },
                featured: vec![
                    Featured::new("amount_usd", "Amount", "Dollars across its levels", "${}"),
                    Featured::new("direction", "Direction", "Neutral starts flat", "{}"),
                    Featured::new("range", "Range", "How wide it spreads", "{} range"),
                ],
            },
            settings: crate::config::settings(),
            supports_paper: true,
            has_loss_stop: true,
            exchanges: vec![ExchangeNeed {
                role: "venue",
                setting: Some("exchange"),
                default: "bullet",
                tradable: true,
                required: true,
            }],
            presets: vec![SOL_GRID],
        })
    }
}

/// A neutral grid of the whole amount over a range set from the market's volatility: its worst
/// case (every buy below the price filled) is the amount, and a hard stop half a range below
/// closes it.
const SOL_GRID: Preset = Preset {
    id: "sol-grid",
    title: "SOL grid (neutral)",
    symbol: Some("SOL-USD"),
    what: "Rests buy orders below the SOL price and sell orders above it across a range set from \
           how much SOL moves, and sells each buy one level higher.",
    money: "Earns on each swing back and forth inside the range; loses when SOL trends through it.",
    amounts: Amounts { min: 50, default: 100, max: 5_000 },
    needs_market: false,
    settings: sol_grid,
    worst_case: |amount| {
        format!(
            "Worst case: SOL falls through the range with every buy filled, about {} held, until \
             the stop half a range below sells it all.",
            money(amount)
        )
    },
};

#[expect(clippy::unnecessary_wraps, reason = "a preset's sizing may refuse; this one never does")]
fn sol_grid(amount: Decimal, _: Option<PresetMarket>) -> Result<BTreeMap<String, String>, String> {
    Ok(BTreeMap::from([
        ("amount_usd".into(), text(amount)),
        ("direction".into(), "neutral".into()),
        ("range".into(), "normal".into()),
    ]))
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
