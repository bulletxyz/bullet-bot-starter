//! The settings: bullet-bots' `[strategies.grid]` (`GridConfig`), field for field, with its
//! defaults and checks. What differs comes from how a bot runs a plugin:
//!
//! - `symbol` is optional: the bot takes it out of a plugin's settings and trades the view's.
//! - `max_position` is the bot's own limit: it takes it out of a plugin's settings. It stays here,
//!   optional, so the grid decides as the built-in does when it is given (the parity tests do).
//! - The range and the levels cannot be derived: a plugin gets no past candles. `lower_price`,
//!   `upper_price` and `num_levels` are required (the built-in, given all three, reads no history
//!   either).
//! - The descriptor declares only what a bot can hand a plugin unchanged. The bot writes every
//!   declared setting the owner left out with its default, so an optional override with no
//!   default (`anchor_price`, `order_size`, `fees`, `trend_filter`) is not declared: the owner can
//!   still set it in the config file, but not by a live change.

use bb_strategy_sdk::{
    Decimal, Loosening, OrderType, Range, SettingKind, SettingSpec, StrategyError, Tier,
};
use serde::{Deserialize, Serialize};

use crate::derive::{Direction, RangePreset};

/// Configuration for the grid, as JSON.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GridConfig {
    /// Broker to trade against: always `"bullet"`.
    #[serde(default = "default_exchange")]
    pub exchange: String,

    /// Trading symbol; the bot gives it in the view, so it is only checked when given.
    #[serde(default)]
    pub symbol: Option<String>,

    /// The dollars the grid trades: its worst case (every level on the losing side filled) is
    /// held to them.
    #[serde(default = "default_amount")]
    pub amount_usd: Decimal,

    /// `neutral` starts flat; `long` / `short` open the position the far side's levels close.
    #[serde(default)]
    pub direction: Direction,

    /// How wide a derived range is (unused by the port: its range is never derived).
    #[serde(default)]
    pub range: RangePreset,

    /// Lower bound of the grid (absolute price).
    #[serde(default)]
    pub lower_price: Option<Decimal>,

    /// Upper bound of the grid (absolute price).
    #[serde(default)]
    pub upper_price: Option<Decimal>,

    /// Levels across `[lower_price, upper_price]`, at least 2.
    #[serde(default)]
    pub num_levels: Option<usize>,

    /// Buys at or below it, sells above. Defaults to the mid at start.
    #[serde(default)]
    pub anchor_price: Option<Decimal>,

    /// Order quantity per level; sized from the dollars when omitted.
    #[serde(default)]
    pub order_size: Option<Decimal>,

    /// `"post_only"` (default: a level that would cross goes at the touch), `"limit"`, `"market"`.
    #[serde(default = "default_order_type")]
    pub order_type: OrderType,

    /// The grid's own position cap (base units): the bot's limit (see the module doc).
    #[serde(default)]
    pub max_position: Option<Decimal>,

    /// The hard stop's distance past the losing edge, in range widths.
    #[serde(default = "default_stop_beyond")]
    pub stop_beyond: Decimal,

    /// Optional fee-floor guardrail: refuse to run when level spacing can't cover
    /// `min_spacing_fee_multiplier × round-trip maker fees`.
    #[serde(default)]
    pub fees: Option<GridFees>,

    /// Optional EMA-slope trend filter: past `pause_divergence_bps` the opening orders are
    /// cancelled and the closing ones stay.
    #[serde(default)]
    pub trend_filter: Option<TrendFilterConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GridFees {
    pub maker_bps: Decimal,
    #[serde(default)]
    pub taker_bps: Decimal,
    #[serde(default = "default_min_spacing_fee_multiplier")]
    pub min_spacing_fee_multiplier: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrendFilterConfig {
    #[serde(default = "default_fast_secs")]
    pub fast_secs: u64,
    #[serde(default = "default_slow_secs")]
    pub slow_secs: u64,
    pub pause_divergence_bps: Decimal,
}

/// The most levels a plugin grid may have: a bot takes at most 64 intents from a plugin in one
/// call, and the start places every level (and the starting IOC) at once. The built-in takes up
/// to 100 given levels.
pub const MAX_PLUGIN_LEVELS: usize = 63;

/// The dollars a grid trades when the section names none.
pub const DEFAULT_AMOUNT_USD: Decimal = Decimal::from_parts(100, 0, 0, false, 0);
/// Half a range width past the losing edge.
pub const DEFAULT_STOP_BEYOND: Decimal = Decimal::from_parts(5, 0, 0, false, 1);

fn default_order_type() -> OrderType {
    OrderType::PostOnly
}

fn default_exchange() -> String {
    "bullet".to_string()
}

fn default_amount() -> Decimal {
    DEFAULT_AMOUNT_USD
}

fn default_stop_beyond() -> Decimal {
    DEFAULT_STOP_BEYOND
}

fn default_min_spacing_fee_multiplier() -> Decimal {
    Decimal::from(3)
}

fn default_fast_secs() -> u64 {
    300
}

fn default_slow_secs() -> u64 {
    3600
}

impl GridConfig {
    /// The market, for the text the grid words: the settings', else the view's once started.
    #[must_use]
    pub fn symbol(&self) -> &str {
        self.symbol.as_deref().unwrap_or_default()
    }

    /// Whether the start needs the market's price history: the range or the levels are derived.
    #[must_use]
    pub fn needs_history(&self) -> bool {
        self.lower_price.is_none() || self.upper_price.is_none() || self.num_levels.is_none()
    }

    /// bullet-bots' `GridConfig::validate`.
    ///
    /// # Errors
    /// The first setting it cannot run with.
    pub fn validate(&self) -> Result<(), String> {
        if self.exchange != "bullet" {
            return Err(format!(
                "exchange {:?}: the grid trades on Bullet only, so exchange must be \"bullet\"",
                self.exchange
            ));
        }
        if self.symbol.as_deref().is_some_and(|s| s.trim().is_empty()) {
            return Err("symbol must be set".to_string());
        }
        if self.amount_usd <= Decimal::ZERO {
            return Err("amount_usd must be > 0".to_string());
        }
        if self.num_levels.is_some_and(|n| n < 2) {
            return Err("num_levels must be ≥ 2".to_string());
        }
        if let (Some(lower), Some(upper)) = (self.lower_price, self.upper_price)
            && lower >= upper
        {
            return Err(format!("lower_price ({lower}) must be < upper_price ({upper})"));
        }
        if self.lower_price.is_some_and(|l| l <= Decimal::ZERO) {
            return Err("lower_price must be > 0".to_string());
        }
        if self.order_size.is_some_and(|q| q <= Decimal::ZERO) {
            return Err("order_size must be > 0".to_string());
        }
        if self.max_position.is_some_and(|q| q <= Decimal::ZERO) {
            return Err("max_position must be > 0".to_string());
        }
        if self.stop_beyond <= Decimal::ZERO {
            return Err("stop_beyond must be > 0: the hard stop is always on".to_string());
        }
        if let (Some(anchor), Some(lower), Some(upper)) =
            (self.anchor_price, self.lower_price, self.upper_price)
            && (anchor < lower || anchor > upper)
        {
            return Err(format!("anchor_price ({anchor}) must be within [{lower}, {upper}]"));
        }
        Ok(())
    }

    /// The settings a plugin can run with: [`Self::validate`], and a range and level count given
    /// (the built-in derives them from past candles, which a plugin does not get).
    ///
    /// # Errors
    /// What is wrong, in plain words.
    pub fn check(&self) -> Result<(), StrategyError> {
        self.validate().map_err(|e| StrategyError(format!("grid: {e}")))?;
        if self.num_levels.is_some_and(|n| n > MAX_PLUGIN_LEVELS) {
            return Err(StrategyError(format!(
                "grid: num_levels must be at most {MAX_PLUGIN_LEVELS}: a bot takes at most 64 \
                 orders from a plugin in one call, and the grid places its levels at once"
            )));
        }
        if self.needs_history() {
            return Err(StrategyError(
                "grid: set lower_price, upper_price and num_levels: a plugin gets no past prices \
                 to derive the range from"
                    .into(),
            ));
        }
        Ok(())
    }
}

fn whole(n: i64) -> Decimal {
    Decimal::from(n)
}

fn choice(path: &str, names: &[&str], default: u32, help: &str) -> SettingSpec {
    let last = i64::try_from(names.len().saturating_sub(1)).unwrap_or(0);
    SettingSpec {
        kind: SettingKind::Choice(names.iter().map(|n| (*n).to_string()).collect()),
        ..SettingSpec::int(path, default, Range::new(whole(0), whole(last)), help)
    }
}

/// The built-in's settings a bot can hand a plugin unchanged (see the module doc), with the
/// built-in's defaults, ranges, tiers and loosening.
#[must_use]
pub fn settings() -> Vec<SettingSpec> {
    let any = Range::default();
    vec![
        SettingSpec {
            guarded: Range::new(whole(10), whole(5_000)),
            ..SettingSpec::decimal(
                "amount_usd",
                DEFAULT_AMOUNT_USD,
                any,
                "Dollars across all its levels: the most it holds with every losing-side level \
                 filled.",
            )
        }
        .tier(Tier::Limit)
        .loosening(Loosening::Up),
        choice(
            "direction",
            Direction::NAMES,
            0,
            "neutral starts flat; long buys the levels above the price at start, short sells the \
             ones below.",
        )
        .tier(Tier::Structural),
        SettingSpec::decimal("lower_price", whole(1), any, "Lower bound of the grid.")
            .tier(Tier::Limit)
            .loosening(Loosening::Down)
            .required(),
        SettingSpec::decimal("upper_price", whole(2), any, "Upper bound of the grid.")
            .tier(Tier::Limit)
            .loosening(Loosening::Up)
            .required(),
        SettingSpec {
            hard: Range::new(whole(2), Decimal::from(MAX_PLUGIN_LEVELS)),
            ..SettingSpec::int(
                "num_levels",
                11,
                Range::new(whole(2), whole(50)),
                "Levels across [lower_price, upper_price].",
            )
        }
        .tier(Tier::Limit)
        .loosening(Loosening::Up)
        .required(),
        choice("order_type", &["limit", "post_only", "market"], 1, "Order type for grid orders.")
            .tier(Tier::Structural),
        SettingSpec {
            hard: Range::new(Decimal::new(5, 2), Decimal::TWO),
            ..SettingSpec::decimal(
                "stop_beyond",
                DEFAULT_STOP_BEYOND,
                Range::new(Decimal::new(1, 1), Decimal::ONE),
                "The hard stop's distance past the losing edge, in range widths: it closes \
                 everything and stops there.",
            )
        }
        .tier(Tier::Limit)
        .loosening(Loosening::Up),
    ]
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
