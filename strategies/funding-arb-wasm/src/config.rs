//! The settings: bullet-bots' `[strategies.funding-arb]` tunables (`FundingArbConfig`), with the
//! names, defaults and ranges it has, and one of this plugin's own (`slippage_bps`).
//!
//! What a plugin cannot take (see the crate docs): `dry_run` (a bot runs a plugin on paper until
//! its owner signs it live), `hedge_exchange` (the hedge leg is declared on `hyperliquid`),
//! `exclude` / `only` (one symbol per instance: the bot's own, with the hedge leg's symbol).
//! `max_positions` is kept for `decide`'s sake; with one symbol it only matters at 0 (rejected).

use bb_strategy_sdk::{Decimal, Loosening, Range, SettingSpec, StrategyError, Tier};
use serde::{Deserialize, Serialize};

use crate::decide::Params;

/// The strategy's settings, as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FundingArbSettings {
    /// Target notional per leg (USD).
    #[serde(default = "d_notional")]
    pub notional_per_symbol: Decimal,
    /// Maximum concurrently open symbols (one symbol per instance here).
    #[serde(default = "d_max_positions")]
    pub max_positions: usize,
    /// Cap on position size as a fraction of Bullet's open interest.
    #[serde(default = "d_oi_pct")]
    pub max_oi_pct: Decimal,
    /// Cap on account leverage per venue.
    #[serde(default = "d_leverage")]
    pub max_leverage: Decimal,
    /// Funding history window used for the signal, hours.
    #[serde(default = "d_window")]
    pub signal_window_hours: u64,
    /// Hold time assumed when amortising fees against funding, hours.
    #[serde(default = "d_hold")]
    pub expected_hold_hours: u64,
    /// How far past the touch an IOC may fill (bps). This plugin's own.
    #[serde(default = "d_slippage")]
    pub slippage_bps: Decimal,
}

impl FundingArbSettings {
    #[must_use]
    pub fn params(&self) -> Params {
        Params {
            notional_per_symbol: self.notional_per_symbol,
            max_positions: self.max_positions,
            max_oi_pct: self.max_oi_pct,
            max_leverage: self.max_leverage,
            expected_hold_hours: self.expected_hold_hours,
        }
    }

    /// The built-in's `FundingArbConfig::validate` for these tunables, and a check of `slippage_bps`.
    ///
    /// # Errors
    /// The first setting it cannot run with.
    pub fn validate(&self) -> Result<(), StrategyError> {
        let bad = |why: &str| Err(StrategyError(format!("funding-arb: {why}")));
        if self.notional_per_symbol <= Decimal::ZERO {
            return bad("notional_per_symbol must be positive");
        }
        if !(1..=50).contains(&self.max_positions) {
            return bad("max_positions must be between 1 and 50");
        }
        if self.max_oi_pct <= Decimal::ZERO || self.max_oi_pct > Decimal::new(5, 1) {
            return bad("max_oi_pct must be in (0, 0.5]");
        }
        if self.max_leverage <= Decimal::ZERO || self.max_leverage > Decimal::from(5) {
            return bad("max_leverage must be in (0, 5]");
        }
        if !(1..=480).contains(&self.signal_window_hours) {
            return bad("signal_window_hours must be between 1 and 480");
        }
        if !(1..=720).contains(&self.expected_hold_hours) {
            return bad("expected_hold_hours must be between 1 and 720");
        }
        if self.slippage_bps <= Decimal::ZERO || self.slippage_bps > Decimal::ONE_HUNDRED {
            return bad("slippage_bps must be in (0, 100]");
        }
        Ok(())
    }
}

fn d_notional() -> Decimal {
    Decimal::from(100)
}

fn d_max_positions() -> usize {
    8
}

fn d_oi_pct() -> Decimal {
    Decimal::new(10, 2)
}

fn d_leverage() -> Decimal {
    Decimal::from(2)
}

fn d_window() -> u64 {
    24
}

fn d_hold() -> u64 {
    72
}

fn d_slippage() -> Decimal {
    Decimal::from(10)
}

fn whole(n: i64) -> Decimal {
    Decimal::from(n)
}

/// An open range below `max` (the built-in's "positive" check is `validate`'s).
fn up_to(max: Decimal) -> Range {
    Range { min: None, max: Some(max) }
}

/// The built-in's `PARAM_SPECS` for these tunables, in its order, plus `slippage_bps`.
#[must_use]
pub fn settings() -> Vec<SettingSpec> {
    vec![
        SettingSpec::decimal(
            "notional_per_symbol",
            whole(100),
            Range::default(),
            "Target notional per leg (USD).",
        )
        .tier(Tier::Limit)
        .loosening(Loosening::Up),
        SettingSpec::int(
            "max_positions",
            8,
            Range::new(whole(1), whole(50)),
            "Maximum concurrently open symbols (one symbol per instance here).",
        )
        .tier(Tier::Limit)
        .loosening(Loosening::Up),
        SettingSpec::decimal(
            "max_oi_pct",
            Decimal::new(10, 2),
            up_to(Decimal::new(5, 1)),
            "Cap on position size as a fraction of Bullet's open interest.",
        )
        .tier(Tier::Limit)
        .loosening(Loosening::Up),
        SettingSpec::decimal(
            "max_leverage",
            whole(2),
            up_to(whole(5)),
            "Cap on account leverage per venue.",
        )
        .tier(Tier::Limit)
        .loosening(Loosening::Up),
        SettingSpec::int(
            "signal_window_hours",
            24,
            Range::new(whole(1), whole(480)),
            "Funding history window used for the signal, hours.",
        ),
        // A longer assumed hold lowers the entry hurdle: past the default only the owner signs.
        SettingSpec {
            guarded: Range::new(whole(1), whole(72)),
            hard: Range::new(whole(1), whole(720)),
            ..SettingSpec::int(
                "expected_hold_hours",
                72,
                Range::new(whole(1), whole(720)),
                "Hold time assumed when amortising fees against funding, hours.",
            )
        }
        .loosening(Loosening::Up),
        SettingSpec::decimal(
            "slippage_bps",
            whole(10),
            up_to(Decimal::ONE_HUNDRED),
            "How far past the touch an IOC may fill (bps).",
        )
        .tier(Tier::Limit)
        .loosening(Loosening::Up),
    ]
}
