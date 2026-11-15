//! The settings: bullet-bots' `[strategies.rsi-momentum]` (`RsiMomentumConfig`), field for field,
//! with the defaults and checks it has. Three differences, all from how a bot runs a plugin:
//!
//! - `symbol` is optional: the bot takes it out of a plugin's settings and trades the view's.
//! - `max_position`, `daily_loss_usd` and `stale_secs` are the bot's own limits: it takes them out
//!   of a plugin's settings and enforces them itself (its order gate, its daily loss stop, its
//!   stale-book stop). They stay here, optional, so the strategy decides as the built-in does when
//!   they are given (the parity tests give them); without them the entry is capped by the bot's
//!   gate only, and marks count as stale after the built-in's default 30 s.
//! - The descriptor declares none of the four, and no `about`/risk/featured block: the WIT world's
//!   descriptor has no place for those.

use bb_strategy_sdk::{Decimal, Loosening, Range, SettingKind, SettingSpec, StrategyError, Tier};
use serde::{Deserialize, Serialize};

use crate::candles::Interval;

/// The strategy's settings, as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RsiMomentumSettings {
    /// The market traded; the bot gives it in the view, so it is only checked when given.
    #[serde(default)]
    pub symbol: Option<String>,
    /// Candle length.
    #[serde(default = "d_interval")]
    pub interval: Interval,
    /// RSI lookback in candles (Wilder smoothing).
    #[serde(default = "d_period")]
    pub rsi_period: u32,
    /// Go long when the RSI crosses above this.
    #[serde(default = "d_long_above")]
    pub long_above: Decimal,
    /// Go short (with `allow_short`) when the RSI crosses below this.
    #[serde(default = "d_short_below")]
    pub short_below: Decimal,
    /// A long exits when the RSI falls below this, a short when it rises above it.
    #[serde(default = "d_exit_band")]
    pub exit_band: Decimal,
    /// Take short entries too. Off: long only.
    #[serde(default)]
    pub allow_short: bool,
    /// Entry size, USD at the mark.
    #[serde(default = "d_notional")]
    pub notional_usd: Decimal,
    /// Cap on the position, base units: the bot's limit (see the module doc).
    #[serde(default)]
    pub max_position: Option<Decimal>,
    /// Close when the price moves this fraction against the entry (checked on every mark).
    #[serde(default = "d_stop")]
    pub stop_loss_pct: Decimal,
    /// The bot's daily loss stop (see the module doc); checked only.
    #[serde(default)]
    pub daily_loss_usd: Option<Decimal>,
    /// Closed candles after an exit before a new entry.
    #[serde(default = "d_cooldown")]
    pub cooldown_candles: u32,
    /// How far past the touch an IOC may fill (bps).
    #[serde(default = "d_slippage")]
    pub slippage_bps: Decimal,
    /// No entries on a mark older than this (the bot's limit, see the module doc).
    #[serde(default = "d_stale")]
    pub stale_secs: u64,
}

impl RsiMomentumSettings {
    #[must_use]
    pub fn params(&self) -> crate::decide::Params {
        crate::decide::Params {
            long_above: self.long_above,
            short_below: self.short_below,
            exit_band: self.exit_band,
            allow_short: self.allow_short,
        }
    }

    /// The position cap entries are sized within: the given one, else none of the strategy's own.
    #[must_use]
    pub fn position_cap(&self) -> Decimal {
        self.max_position.unwrap_or(Decimal::MAX)
    }

    /// bullet-bots' `RsiMomentumConfig::validate`, with `max_position` optional.
    ///
    /// # Errors
    /// The first setting it cannot run with.
    pub fn validate(&self) -> Result<(), StrategyError> {
        let bad = |why: &str| Err(StrategyError(format!("rsi-momentum: {why}")));
        if self.symbol.as_deref().is_some_and(|s| s.trim().is_empty()) {
            return bad("symbol must be set");
        }
        if !(2..=100).contains(&self.rsi_period) {
            return bad("rsi_period must be between 2 and 100");
        }
        for (name, v) in [
            ("long_above", self.long_above),
            ("short_below", self.short_below),
            ("exit_band", self.exit_band),
        ] {
            if v < Decimal::ZERO || v > Decimal::ONE_HUNDRED {
                return bad(&format!("{name} must be in [0, 100]"));
            }
        }
        if self.short_below >= self.long_above {
            return bad("short_below must be below long_above");
        }
        if self.exit_band < self.short_below || self.exit_band > self.long_above {
            return bad(
                "exit_band must lie between short_below and long_above (else a fresh entry would \
                 exit at once)",
            );
        }
        if self.notional_usd <= Decimal::ZERO {
            return bad("notional_usd must be positive");
        }
        if self.max_position.is_some_and(|v| v <= Decimal::ZERO) {
            return bad("max_position must be positive");
        }
        if self.stop_loss_pct <= Decimal::ZERO || self.stop_loss_pct > Decimal::new(5, 1) {
            return bad("stop_loss_pct must be in (0, 0.5]");
        }
        if self.daily_loss_usd.is_some_and(|v| v <= Decimal::ZERO) {
            return bad("daily_loss_usd must be positive");
        }
        if self.cooldown_candles > 100 {
            return bad("cooldown_candles must be at most 100");
        }
        if self.slippage_bps <= Decimal::ZERO || self.slippage_bps > Decimal::ONE_HUNDRED {
            return bad("slippage_bps must be in (0, 100]");
        }
        if !(5..=600).contains(&self.stale_secs) {
            return bad("stale_secs must be between 5 and 600");
        }
        Ok(())
    }
}

fn d_interval() -> Interval {
    Interval::M15
}

fn d_period() -> u32 {
    14
}

fn d_long_above() -> Decimal {
    Decimal::from(60)
}

fn d_short_below() -> Decimal {
    Decimal::from(40)
}

fn d_exit_band() -> Decimal {
    Decimal::from(50)
}

fn d_notional() -> Decimal {
    Decimal::from(50)
}

fn d_stop() -> Decimal {
    Decimal::new(2, 2)
}

fn d_cooldown() -> u32 {
    2
}

fn d_slippage() -> Decimal {
    Decimal::from(10)
}

fn d_stale() -> u64 {
    30
}

fn whole(n: i64) -> Decimal {
    Decimal::from(n)
}

fn range(min: i64, max: i64) -> Range {
    Range::new(whole(min), whole(max))
}

/// An open range below `max` (the built-in's "positive" check is `validate`'s).
fn up_to(max: Decimal) -> Range {
    Range { min: None, max: Some(max) }
}

/// A decimal setting whose guarded range is narrower than its hard one.
fn banded(path: &str, default: i64, guarded: Range, help: &str) -> SettingSpec {
    SettingSpec { guarded, ..SettingSpec::decimal(path, whole(default), range(0, 100), help) }
}

/// bullet-bots' `PARAM_SPECS` for rsi-momentum, in its order, without the bot's own limits
/// (`daily_loss_usd`, `stale_secs`; see the module doc).
#[must_use]
pub fn settings() -> Vec<SettingSpec> {
    let interval = SettingSpec {
        kind: SettingKind::Choice(Interval::NAMES.iter().map(|n| (*n).to_string()).collect()),
        ..SettingSpec::int("interval", 2, range(0, 6), "Candle length.")
    }
    .tier(Tier::Structural);
    let allow_short = SettingSpec {
        kind: SettingKind::Flag,
        ..SettingSpec::int(
            "allow_short",
            0,
            range(0, 1),
            "Take short entries too; off is long only.",
        )
    }
    .tier(Tier::Limit)
    .loosening(Loosening::Up);
    vec![
        interval,
        SettingSpec::int(
            "rsi_period",
            14,
            range(2, 100),
            "RSI lookback in candles (Wilder smoothing).",
        )
        .tier(Tier::Structural),
        banded("long_above", 60, range(55, 90), "Go long when the RSI crosses above this.")
            .loosening(Loosening::Down),
        banded(
            "short_below",
            40,
            range(10, 45),
            "Go short (with allow_short) when the RSI crosses below this.",
        )
        .loosening(Loosening::Up),
        banded(
            "exit_band",
            50,
            range(40, 60),
            "A long exits when the RSI falls below this, a short when it rises above it.",
        ),
        allow_short,
        SettingSpec::decimal(
            "notional_usd",
            whole(50),
            Range::default(),
            "Entry size, USD at the mark.",
        )
        .tier(Tier::Limit)
        .loosening(Loosening::Up),
        SettingSpec::decimal(
            "stop_loss_pct",
            Decimal::new(2, 2),
            up_to(Decimal::new(5, 1)),
            "Close when the price moves this fraction against the entry (always on).",
        )
        .tier(Tier::Limit)
        .loosening(Loosening::Up),
        SettingSpec::int(
            "cooldown_candles",
            2,
            range(0, 100),
            "Closed candles after an exit before a new entry.",
        )
        .tier(Tier::Limit)
        .loosening(Loosening::Down),
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

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
