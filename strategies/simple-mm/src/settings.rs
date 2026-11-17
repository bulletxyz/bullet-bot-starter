//! The market maker's section (`[strategies.simple-mm]`) as the bot hands it over, and its checks.

use std::collections::BTreeMap;

use bb_strategy_sdk::param_spec::BoundsMode;
use bb_strategy_sdk::{OrderType, ValidateConfig};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::params::{FairValue, MmParams};

mod specs;

pub use specs::{BUILTIN_SPECS, PARAMS, VOL_THRESHOLD_BPS, VOL_WIDEN_BPS, VOL_WIDEN_ENABLED};
pub use specs::{VOL_WINDOW_SECS, settings};

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarketMakerSettings {
    #[serde(default = "default_exchange")]
    pub exchange: String,
    /// Empty: no market yet (the bot waits for setup to pick one).
    #[serde(default)]
    pub symbol: String,
    /// Bid distance below the fair value, in basis points.
    pub bid_spread_bps: Decimal,
    /// Ask distance above the fair value, in basis points.
    pub ask_spread_bps: Decimal,
    /// The size of each quote.
    pub order_size: Decimal,
    /// The most net position the quotes may build, either way.
    pub max_position: Decimal,
    #[serde(default = "default_refresh_secs")]
    pub refresh_secs: u64,
    /// A resting quote is replaced only once its price is this far from the wanted one.
    #[serde(default = "default_refresh_threshold_bps")]
    pub refresh_threshold_bps: Decimal,
    #[serde(default = "default_order_type")]
    pub order_type: OrderType,
    /// The fee floor: both spreads together must cover this many round-trip maker fees.
    #[serde(default)]
    pub fees: Option<Fees>,
    /// Inventory skew at the position cap, in bps of both quotes (0: none).
    #[serde(default)]
    pub skew_bps: Decimal,
    #[serde(default)]
    pub daily_loss_usd: Option<Decimal>,
    #[serde(default)]
    pub stale_secs: Option<u64>,
    #[serde(default = "default_max_orders_per_min")]
    pub max_orders_per_min: u32,
    /// The most notional (USD) the quotes may build, either way.
    #[serde(default)]
    pub max_notional_usd: Option<Decimal>,
    #[serde(default)]
    pub fair_value: FairValue,
    #[serde(default = "default_basis_halflife_secs")]
    pub basis_halflife_secs: u64,
    #[serde(default = "default_reference_stale_secs")]
    pub reference_stale_secs: u64,
    #[serde(default = "default_max_divergence_bps")]
    pub max_divergence_bps: Decimal,
    /// The Binance USD-M perp for the fair value; the base asset plus `USDT` when unset.
    #[serde(default)]
    pub binance_symbol: Option<String>,
    /// Starting values of the strategy's own params ([`PARAMS`]); a switch may be written as a
    /// flag.
    #[serde(default, deserialize_with = "extra_values")]
    pub extra: BTreeMap<String, Decimal>,
    // Keys a v51 section holds that are no longer the strategy's: the mode and the state
    // directory are the instance's, the in-flight cap and the guards the host's. Accepted so
    // a v51 section loads; never read, the in-flight cap only held to the range v51 accepted.
    #[serde(default, rename = "dry_run")]
    #[schemars(skip)]
    _dry_run: IgnoredAny,
    #[serde(default, rename = "state_dir")]
    #[schemars(skip)]
    _state_dir: IgnoredAny,
    #[serde(default, rename = "max_inflight")]
    #[schemars(skip)]
    max_inflight: Option<u64>,
    #[serde(default, rename = "guards")]
    #[schemars(skip)]
    _guards: IgnoredAny,
    #[serde(default, rename = "max_position_fallback")]
    #[schemars(skip)]
    _max_position_fallback: IgnoredAny,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fees {
    pub maker_bps: Decimal,
    #[serde(default = "default_min_spread_fee_multiplier")]
    pub min_spread_fee_multiplier: Decimal,
}

/// A value of `extra`: a number, or a flag (1 on, 0 off).
#[derive(Deserialize)]
#[serde(untagged)]
enum ExtraValue {
    Flag(bool),
    Number(Decimal),
}

fn extra_values<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, Decimal>, D::Error> {
    let values = BTreeMap::<String, ExtraValue>::deserialize(d)?;
    Ok(values
        .into_iter()
        .map(|(k, v)| match v {
            ExtraValue::Flag(on) => (k, Decimal::from(u8::from(on))),
            ExtraValue::Number(n) => (k, n),
        })
        .collect())
}

fn default_exchange() -> String {
    "bullet".to_string()
}

fn default_refresh_secs() -> u64 {
    5
}

fn default_refresh_threshold_bps() -> Decimal {
    Decimal::ONE
}

fn default_order_type() -> OrderType {
    OrderType::PostOnly
}

/// Far above the market maker's two placements per refresh.
pub const DEFAULT_MAX_ORDERS_PER_MIN: u32 = 600;

/// The largest in-flight cap a v51 section may hold.
pub const MAX_INFLIGHT: u64 = 2;

fn default_max_orders_per_min() -> u32 {
    DEFAULT_MAX_ORDERS_PER_MIN
}

fn default_basis_halflife_secs() -> u64 {
    300
}

fn default_reference_stale_secs() -> u64 {
    5
}

fn default_max_divergence_bps() -> Decimal {
    Decimal::from(20)
}

fn default_min_spread_fee_multiplier() -> Decimal {
    Decimal::from(3)
}

impl ValidateConfig for MarketMakerSettings {
    fn validate(&self) -> Result<(), String> {
        let positive = |v: Decimal, name: &str| {
            if v > Decimal::ZERO { Ok(()) } else { Err(format!("{name} must be positive")) }
        };
        positive(self.bid_spread_bps, "bid_spread_bps")?;
        positive(self.ask_spread_bps, "ask_spread_bps")?;
        positive(self.order_size, "order_size")?;
        if self.max_position < self.order_size {
            return Err(format!(
                "the largest position {} is below the order size {}. Lower the order size, or \
                 set a larger position",
                self.max_position, self.order_size
            ));
        }
        if self.refresh_secs == 0 {
            return Err("refresh_secs must be >= 1".into());
        }
        if self.refresh_threshold_bps < Decimal::ZERO {
            return Err("refresh_threshold_bps must be non-negative".into());
        }
        if self.skew_bps < Decimal::ZERO || self.skew_bps > Decimal::ONE_HUNDRED {
            return Err("skew_bps must be between 0 and 100".into());
        }
        if let Some(loss) = self.daily_loss_usd {
            positive(loss, "daily_loss_usd")?;
        }
        if self.stale_secs == Some(0) {
            return Err("stale_secs must be >= 1".into());
        }
        if self.max_orders_per_min == 0 {
            return Err("max_orders_per_min must be >= 1".into());
        }
        if self.max_inflight.is_some_and(|n| !(1..=MAX_INFLIGHT).contains(&n)) {
            return Err(format!("max_inflight must be between 1 and {MAX_INFLIGHT}"));
        }
        if let Some(cap) = self.max_notional_usd {
            positive(cap, "max_notional_usd")?;
        }
        if self.binance_symbol.as_deref().is_some_and(|s| s.trim().is_empty()) {
            return Err("binance_symbol must not be blank".into());
        }
        // The ranges a live change of these is held to.
        let params = MmParams::from_settings(self);
        for name in ["basis_halflife_secs", "reference_stale_secs", "max_divergence_bps"] {
            params.check(name, BoundsMode::Guarded, None)?;
        }
        self.check_fee_floor()
    }
}

impl MarketMakerSettings {
    /// The Binance symbol for the fair value, upper case (e.g. `BTCUSDT`).
    #[must_use]
    pub fn binance_symbol(&self) -> String {
        self.binance_symbol
            .clone()
            .unwrap_or_else(|| {
                format!("{}USDT", self.symbol.split('-').next().unwrap_or(&self.symbol))
            })
            .trim()
            .to_ascii_uppercase()
    }

    fn check_fee_floor(&self) -> Result<(), String> {
        let Some(fees) = &self.fees else { return Ok(()) };
        if fees.maker_bps < Decimal::ZERO {
            return Err("fees.maker_bps must be non-negative".into());
        }
        if fees.min_spread_fee_multiplier < Decimal::ONE {
            return Err("fees.min_spread_fee_multiplier must be >= 1".into());
        }
        let overflow = || "the spreads and fees are too large to check the fee floor".to_string();
        let total = self.bid_spread_bps.checked_add(self.ask_spread_bps).ok_or_else(overflow)?;
        let required = Decimal::TWO
            .checked_mul(fees.maker_bps)
            .and_then(|v| v.checked_mul(fees.min_spread_fee_multiplier))
            .ok_or_else(overflow)?;
        if total < required {
            return Err(format!(
                "total spread {total} bps < required {required} bps \
                 (2 * maker_bps * min_spread_fee_multiplier)"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
