//! The settings: bullet-bots' `[strategies.avellaneda-stoikov]` (`AvellanedaStoikovConfig`), field
//! for field, with the defaults and checks it has. What differs, all from how a bot runs a plugin:
//!
//! - `symbol` is optional: the bot gives the view's market, so it is only checked when given.
//!   `exchange` is accepted for the same settings file and must be `bullet`.
//! - the reference feed is the leg `reference` (see [`crate::REFERENCE_LEG`]): `reference_exchange`
//!   only switches the fair-value mode on and `reference_symbol` picks the leg's symbol. Which
//!   Binance market feeds it (`reference_market`) is the owner's leg configuration; the setting is
//!   kept but nothing here reads it.
//! - `order_levels` is at most [`MAX_LEVELS`]: a bot takes at most 64 intents from a plugin in one
//!   call, and a requote can touch every level on both sides.
//! - the descriptor has no `about`/risk block: the world's descriptor has no place for it. The two
//!   text settings (`exchange`, `symbol`) are not declared: the bot owns the venue and the market.

use bb_strategy_sdk::{
    Decimal, Loosening, OrderType, Range, SettingKind, SettingSpec, StrategyError, Tier,
};
use serde::{Deserialize, Serialize};

/// Most levels per side: a requote makes at most one cancel, amend or place per level and side,
/// plus the reconcile's read, inside the 64 intents a bot takes per call.
pub const MAX_LEVELS: usize = 30;

/// The strategy's settings, as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AvellanedaStoikovSettings {
    /// Must be `bullet` when given.
    #[serde(default)]
    pub exchange: Option<String>,
    /// The market traded; the bot gives it in the view, so it is only checked when given.
    #[serde(default)]
    pub symbol: Option<String>,
    /// Set: the quotes are priced off the `reference` leg's mid instead of the trading venue's.
    #[serde(default)]
    pub reference_exchange: Option<String>,
    /// The reference leg's symbol; required with `reference_exchange`.
    #[serde(default)]
    pub reference_symbol: Option<String>,
    /// Which Binance market the owner points the leg at (informational here).
    #[serde(default = "d_reference_market")]
    pub reference_market: String,
    /// Stop requoting when the reference is older than this (s).
    #[serde(default = "d_reference_stale_secs")]
    pub reference_stale_secs: u64,
    /// Quantity per quote.
    pub order_size: Decimal,
    /// Hard position cap: normalises the inventory skew and stops the leaned side.
    pub max_position: Decimal,
    /// Target inventory, base units.
    #[serde(default)]
    pub inventory_target: Decimal,
    /// Risk aversion γ (> 0).
    pub gamma: Decimal,
    /// Order-flow intensity κ (> 0).
    pub kappa: Decimal,
    /// Finite horizon τ in the closed form (s).
    #[serde(default = "d_order_horizon_secs")]
    pub order_horizon_secs: u64,
    /// Rolling window of the volatility estimator (s).
    #[serde(default = "d_vol_window_secs")]
    pub vol_window_secs: u64,
    /// Requote cadence (s): the first tick past it refreshes the quotes.
    #[serde(default = "d_order_refresh_secs")]
    pub order_refresh_secs: u64,
    /// Half-spread floor (bps).
    #[serde(default = "d_min_half_spread_bps")]
    pub min_half_spread_bps: Decimal,
    /// Half-spread ceiling (bps).
    #[serde(default = "d_max_half_spread_bps")]
    pub max_half_spread_bps: Decimal,
    /// Quote levels per side.
    #[serde(default = "d_order_levels")]
    pub order_levels: usize,
    /// Extra half-spread between levels (bps of mid).
    #[serde(default = "d_order_level_spread_bps")]
    pub order_level_spread_bps: Decimal,
    /// Per-level size multiplier increment.
    #[serde(default)]
    pub order_level_amount_step: Decimal,
    /// Order type for quotes.
    #[serde(default = "d_order_type")]
    pub order_type: OrderType,
    /// Drift before the inner level is amended (bps of mid).
    #[serde(default = "d_amend_threshold_bps")]
    pub amend_threshold_bps: Decimal,
    /// Per-level widening of the amend threshold (bps).
    #[serde(default)]
    pub amend_threshold_step_bps: Decimal,
    /// Minimum gap between refreshes that call the venue (ms); 0 disables.
    #[serde(default = "d_min_refresh_interval_ms")]
    pub min_refresh_interval_ms: u64,
    /// Safety-net reconcile against the venue's open orders (s); 0 disables.
    #[serde(default = "d_reconcile_interval_secs")]
    pub reconcile_interval_secs: u64,
    /// The fee-floor guard: refuses to start when `min_half_spread_bps` cannot cover the fees.
    #[serde(default)]
    pub fees: Option<AsFees>,
}

/// The fee-floor guard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AsFees {
    pub maker_bps: Decimal,
    #[serde(default)]
    pub taker_bps: Decimal,
    /// `min_half_spread_bps × 2` must be at least `multiplier × 2 × maker_bps`.
    #[serde(default = "d_min_spread_fee_multiplier")]
    pub min_spread_fee_multiplier: Decimal,
}

fn d_reference_market() -> String {
    "perp".into()
}
fn d_reference_stale_secs() -> u64 {
    10
}
fn d_order_horizon_secs() -> u64 {
    60
}
fn d_vol_window_secs() -> u64 {
    600
}
fn d_order_refresh_secs() -> u64 {
    10
}
fn d_min_half_spread_bps() -> Decimal {
    Decimal::from(5)
}
fn d_max_half_spread_bps() -> Decimal {
    Decimal::from(500)
}
fn d_order_levels() -> usize {
    3
}
fn d_order_level_spread_bps() -> Decimal {
    Decimal::from(10)
}
fn d_order_type() -> OrderType {
    OrderType::PostOnly
}
fn d_amend_threshold_bps() -> Decimal {
    Decimal::new(5, 1)
}
fn d_min_refresh_interval_ms() -> u64 {
    100
}
fn d_reconcile_interval_secs() -> u64 {
    10
}
fn d_min_spread_fee_multiplier() -> Decimal {
    Decimal::from(3)
}

impl AvellanedaStoikovSettings {
    /// Whether the quotes are priced off the reference leg.
    #[must_use]
    pub fn uses_reference(&self) -> bool {
        self.reference_exchange.is_some()
    }

    /// bullet-bots' `AvellanedaStoikovConfig::validate`, with the plugin's one extra bound.
    ///
    /// # Errors
    /// The first setting it cannot run with.
    pub fn validate(&self) -> Result<(), StrategyError> {
        self.check().map_err(|e| StrategyError(format!("avellaneda-stoikov: {e}")))
    }

    fn check(&self) -> Result<(), String> {
        let zero = Decimal::ZERO;
        if let Some(exchange) = &self.exchange
            && exchange != "bullet"
        {
            return Err(format!(
                "exchange {exchange:?}: the strategy trades on Bullet only, so exchange must be \
                 \"bullet\""
            ));
        }
        if self.symbol.as_deref().is_some_and(|s| s.trim().is_empty()) {
            return Err("symbol must be set".into());
        }
        if self.order_size <= zero {
            return Err("order_size must be positive".into());
        }
        if self.max_position < self.order_size {
            return Err("max_position must be >= order_size".into());
        }
        if self.gamma <= zero {
            return Err("gamma must be positive".into());
        }
        if self.kappa <= zero {
            return Err("kappa must be positive".into());
        }
        if self.order_horizon_secs == 0 {
            return Err("order_horizon_secs must be >= 1".into());
        }
        if self.vol_window_secs == 0 {
            return Err("vol_window_secs must be >= 1".into());
        }
        if self.min_half_spread_bps <= zero {
            return Err("min_half_spread_bps must be positive".into());
        }
        if self.max_half_spread_bps < self.min_half_spread_bps {
            return Err("max_half_spread_bps must be >= min_half_spread_bps".into());
        }
        if self.order_levels == 0 {
            return Err("order_levels must be >= 1".into());
        }
        if self.order_levels > MAX_LEVELS {
            return Err(format!(
                "order_levels must be at most {MAX_LEVELS}: a bot takes at most 64 intents from \
                 a plugin in one call"
            ));
        }
        if self.order_level_spread_bps < zero {
            return Err("order_level_spread_bps must be non-negative".into());
        }
        if self.order_level_amount_step < zero {
            return Err("order_level_amount_step must be non-negative".into());
        }
        if self.amend_threshold_bps < zero {
            return Err("amend_threshold_bps must be non-negative".into());
        }
        if self.amend_threshold_step_bps < zero {
            return Err("amend_threshold_step_bps must be non-negative".into());
        }
        if self.reference_exchange.is_some() && self.reference_symbol.is_none() {
            return Err("reference_exchange requires reference_symbol".into());
        }
        if self.reference_stale_secs == 0 {
            return Err("reference_stale_secs must be >= 1".into());
        }
        if let Some(fees) = &self.fees {
            if fees.maker_bps < zero {
                return Err("fees.maker_bps must be non-negative".into());
            }
            if fees.min_spread_fee_multiplier < Decimal::ONE {
                return Err("fees.min_spread_fee_multiplier must be >= 1".into());
            }
        }
        Ok(())
    }

    /// The fee-floor guard: `min_half_spread_bps × 2` must cover the round-trip maker fee times the
    /// multiplier. No `fees` block: nothing to check.
    ///
    /// # Errors
    /// The floor is not covered, or the product overflows.
    pub fn check_fee_floor(&self) -> Result<(), StrategyError> {
        let Some(fees) = &self.fees else { return Ok(()) };
        let overflow = || StrategyError("avellaneda-stoikov: fee floor overflows".into());
        let round_trip_bps = Decimal::TWO.checked_mul(fees.maker_bps).ok_or_else(overflow)?;
        let required_bps =
            round_trip_bps.checked_mul(fees.min_spread_fee_multiplier).ok_or_else(overflow)?;
        let min_full_spread_bps =
            Decimal::TWO.checked_mul(self.min_half_spread_bps).ok_or_else(overflow)?;
        if min_full_spread_bps < required_bps {
            return Err(StrategyError(format!(
                "min_half_spread_bps × 2 = {min_full_spread_bps} bps is below required \
                 {required_bps} bps ({} × round-trip maker fee). Raise min_half_spread_bps.",
                fees.min_spread_fee_multiplier
            )));
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

fn text(path: &str, default: &str, help: &str) -> SettingSpec {
    SettingSpec {
        kind: SettingKind::Text(default.into()),
        ..SettingSpec::int(path, 0, Range::default(), help)
    }
    .tier(Tier::Structural)
}

/// A setting with `range` as its guarded and hard range, structural.
fn model(path: &str, default: Decimal, range: Range, help: &str) -> SettingSpec {
    SettingSpec::decimal(path, default, range, help).tier(Tier::Structural)
}

fn secs(path: &str, default: u32, range: Range, help: &str) -> SettingSpec {
    SettingSpec::int(path, default, range, help).tier(Tier::Structural)
}

/// The built-in's settings a bot can hand a plugin, with its defaults, ranges, tiers and loosening
/// (see the module doc for the differences).
#[must_use]
#[expect(clippy::too_many_lines, reason = "one declaration per setting")]
pub fn settings() -> Vec<SettingSpec> {
    let any = Range::default();
    let at_least = |n| Range { min: Some(whole(n)), max: None };
    let positive = |max| Range::new(Decimal::new(1, 4), whole(max));
    // A guard's guarded range ends at its default on the loose side, as the built-in declares it.
    let guard = |s: SettingSpec, guarded: Range| SettingSpec { guarded, hard: any, ..s };
    vec![
        text(
            "reference_exchange",
            "binance",
            "Exchange whose mid prices the quotes (the `reference` leg); omit to use the trading \
             venue's.",
        ),
        text(
            "reference_symbol",
            "btcusdt",
            "Symbol on the reference venue; needed with reference_exchange.",
        ),
        choice("reference_market", &["perp", "spot"], 0, "Binance market used as the reference.")
            .tier(Tier::Structural),
        guard(
            secs(
                "reference_stale_secs",
                10,
                any,
                "Stop requoting when the reference is older than this.",
            )
            .loosening(Loosening::Up),
            Range::new(whole(1), whole(10)),
        ),
        SettingSpec::decimal("order_size", whole(1), any, "Quantity per quote.")
            .tier(Tier::Limit)
            .loosening(Loosening::Up)
            .required(),
        SettingSpec::decimal("max_position", whole(1), any, "Hard position cap.")
            .tier(Tier::Limit)
            .loosening(Loosening::Up)
            .required(),
        model(
            "inventory_target",
            whole(0),
            Range::new(whole(-1_000_000), whole(1_000_000)),
            "Target inventory in base units.",
        ),
        model("gamma", whole(1), positive(100), "Risk aversion γ (> 0).").required(),
        model("kappa", whole(1), positive(10_000), "Order-flow intensity κ (> 0).").required(),
        secs(
            "order_horizon_secs",
            60,
            Range::new(whole(1), whole(86_400)),
            "Horizon τ in the closed form (s).",
        ),
        secs(
            "vol_window_secs",
            600,
            Range::new(whole(1), whole(86_400)),
            "Volatility estimator window (s).",
        ),
        guard(
            secs("order_refresh_secs", 10, any, "Requote cadence (s).").loosening(Loosening::Up),
            Range::new(whole(1), whole(60)),
        ),
        guard(
            model("min_half_spread_bps", whole(5), any, "Half-spread floor (bps).")
                .loosening(Loosening::Down),
            at_least(5),
        ),
        model(
            "max_half_spread_bps",
            whole(500),
            Range::new(whole(1), whole(10_000)),
            "Half-spread ceiling (bps).",
        ),
        SettingSpec::int("order_levels", 3, any, "Quote levels per side.")
            .tier(Tier::Limit)
            .loosening(Loosening::Up),
        model(
            "order_level_spread_bps",
            whole(10),
            Range::new(whole(0), whole(1_000)),
            "Extra half-spread between levels (bps).",
        ),
        model(
            "order_level_amount_step",
            whole(0),
            Range::new(whole(0), whole(10)),
            "Per-level size multiplier increment.",
        ),
        choice("order_type", &["limit", "post_only", "market"], 1, "Order type for quotes.")
            .tier(Tier::Structural),
        guard(
            model(
                "amend_threshold_bps",
                Decimal::new(5, 1),
                any,
                "Drift before the inner level is amended (bps).",
            )
            .loosening(Loosening::Up),
            Range::new(whole(0), whole(50)),
        ),
        model(
            "amend_threshold_step_bps",
            whole(0),
            Range::new(whole(0), whole(100)),
            "Per-level widening of the amend threshold (bps).",
        ),
        guard(
            secs(
                "min_refresh_interval_ms",
                100,
                any,
                "Minimum gap between refreshes that call the venue (ms).",
            )
            .loosening(Loosening::Up),
            Range::new(whole(0), whole(1000)),
        ),
        guard(
            secs(
                "reconcile_interval_secs",
                10,
                any,
                "Reconcile with the venue's open orders every this many seconds; 0 disables it.",
            )
            .loosening(Loosening::Up),
            Range::new(whole(1), whole(10)),
        ),
        SettingSpec::decimal(
            "fees.maker_bps",
            whole(0),
            Range::new(whole(0), whole(1000)),
            "Maker fee (bps); the fee floor is built from it, so lowering it loosens the floor.",
        )
        .tier(Tier::Structural)
        .loosening(Loosening::Down),
        model("fees.taker_bps", whole(0), Range::new(whole(0), whole(1_000)), "Taker fee (bps)."),
        guard(
            model(
                "fees.min_spread_fee_multiplier",
                whole(3),
                any,
                "min_half_spread_bps must cover this many round-trip maker fees.",
            )
            .loosening(Loosening::Down),
            at_least(3),
        ),
    ]
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
