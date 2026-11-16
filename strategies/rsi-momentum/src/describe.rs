//! rsi-momentum as the bot registers it: its name, its settings and its build from them.

use std::collections::BTreeMap;

use bb_strategy_sdk::profile::{Amounts, PresetMarket, floor_to, money, pct, text, usable};
use bb_strategy_sdk::{
    About, Describe, Descriptor, ExchangeNeed, Featured, FromSettings, Preset, Profile, Risk,
    RiskLevel, SettingSpec, StrategyError, ValidateConfig,
};
use rust_decimal::Decimal;

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

    fn profile() -> Option<Profile> {
        Some(Profile {
            about: About {
                title: "RSI momentum",
                tagline: "Follows strong moves",
                blurb: "Goes long when momentum turns up and exits when it fades. Long only \
                        unless you allow shorts.",
                risk: Risk {
                    level: RiskLevel::MediumHigh,
                    text: "Directional and unhedged; choppy markets cause small repeated losses.",
                },
                featured: vec![
                    Featured::new("rsi_period", "RSI period", "Candles it looks back", "RSI {}"),
                    Featured::new("long_above", "Go long above", "RSI level", "in above {}"),
                    Featured::new("exit_band", "Exit below", "RSI level", "out below {}"),
                ],
            },
            settings: crate::settings(),
            supports_paper: true,
            has_loss_stop: true,
            exchanges: vec![ExchangeNeed {
                role: "venue",
                setting: None,
                default: "bullet",
                tradable: true,
                required: true,
            }],
            presets: vec![BTC_RSI],
        })
    }
}

/// The day's loss stop of the preset, in percent of the amount.
const LOSS_PCT: u32 = 5;
/// The price-staleness stop the preset sets (the floor needs one to go live).
const STALE_SECS: &str = "30";

/// One directional position of the whole amount, capped at the amount at the market's price.
const BTC_RSI: Preset = Preset {
    id: "btc-rsi-momentum",
    title: "BTC RSI momentum (long only)",
    symbol: Some("BTC-USD"),
    what: "Buys BTC when its 15-minute RSI turns up through 60 and sells when momentum fades (RSI \
           under 50) or the price falls 2% from where it bought.",
    money: "Earns when a move it buys into keeps going; loses in choppy markets, where it buys \
            late and is stopped out.",
    amounts: Amounts { min: 50, default: 100, max: 5_000 },
    needs_market: true,
    settings: btc_rsi,
    worst_case: |amount| {
        format!(
            "Worst day: it stops itself after a {} loss (more only if the price jumps past the \
             stop before it can close).",
            money(pct(amount, LOSS_PCT))
        )
    },
};

fn btc_rsi(
    amount: Decimal,
    market: Option<PresetMarket>,
) -> Result<BTreeMap<String, String>, String> {
    let m = usable(BTC_RSI.title, market)?;
    let cap = floor_to(amount.checked_div(m.price).unwrap_or_default(), m.step);
    if cap < m.step {
        return Err(format!(
            "{} is too small for {} at {}: its smallest order takes about {}",
            money(amount),
            BTC_RSI.symbol.unwrap_or("the market"),
            money(m.price.round_dp(2)),
            money(m.step.checked_mul(m.price).unwrap_or_default().round_dp(2))
        ));
    }
    Ok(BTreeMap::from([
        ("notional_usd".into(), text(amount)),
        ("max_position".into(), text(cap)),
        ("daily_loss_usd".into(), text(pct(amount, LOSS_PCT))),
        ("stale_secs".into(), STALE_SECS.into()),
    ]))
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
