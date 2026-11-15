//! `[strategies.simple-funding]`: the knobs and their declared specs (the owner-page text is the bot's).

use bb_strategy_sdk::param_spec::{Loosening, ParamSpec, Range, Tier, whole};
use bb_strategy_sdk::{Setting, ValidateConfig};
use rust_decimal::Decimal;
use serde::Deserialize;

/// User-facing knobs for the Bullet-only funding carry. Bullet is the only venue.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimpleFundingConfig {
    /// Target notional per leg (USD); every leg is this size.
    #[serde(default = "d_notional")]
    pub notional_per_symbol: Decimal,
    /// Legs per side: short the top `top_k` positive signals, long the bottom `top_k` negative.
    #[serde(default = "d_top_k")]
    pub top_k: usize,
    /// Maximum concurrently open symbols.
    #[serde(default = "d_max_positions")]
    pub max_positions: usize,
    /// The long and short books stay within this share of gross of each other.
    #[serde(default = "d_max_net_pct")]
    pub max_net_pct: Decimal,
    /// Cap on a leg as a fraction of the market's open interest.
    #[serde(default = "d_oi_pct")]
    pub max_oi_pct: Decimal,
    /// Cap on gross notional over account equity.
    #[serde(default = "d_lev")]
    pub max_leverage: Decimal,
    /// Settled-funding window the signal averages.
    #[serde(default = "d_window")]
    pub signal_window_hours: u64,
    /// Hold time assumed when amortising the round trip against funding.
    #[serde(default = "d_hold")]
    pub expected_hold_hours: u64,
    /// How far past the touch an IOC order may fill (bps).
    #[serde(default = "d_slippage")]
    pub slippage_bps: Decimal,
    /// Books thinner than this (USD, the thinner side) are not entered.
    #[serde(default = "d_depth")]
    pub min_depth_usd: Decimal,
    /// Equity drop since the UTC day start (USD) that stops the day. A live run must set it:
    /// absent, [`SimpleFundingConfig::check_live`] refuses it. A paper run without one uses
    /// [`DEFAULT_DAILY_LOSS_USD`].
    #[serde(default)]
    pub daily_loss_usd: Option<Decimal>,
    /// No entries on a market whose mark or book is older than this.
    #[serde(default = "d_stale")]
    pub stale_secs: u64,
    /// Manage (exit, flatten) positions the universe already holds that this bot did not
    /// open. Off: a live start with such a position is refused.
    #[serde(default)]
    pub adopt_existing_positions: bool,
    /// Symbols never traded.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// If non-empty, the only symbols traded.
    #[serde(default)]
    pub only: Vec<String>,
}

/// The loss stop a paper run assumes when none is set. A live run never gets a default.
pub const DEFAULT_DAILY_LOSS_USD: Decimal = Decimal::from_parts(20, 0, 0, false, 0);

impl SimpleFundingConfig {
    /// The daily loss stop in force: the configured one, else the dry-run default.
    #[must_use]
    pub fn loss_stop_usd(&self) -> Decimal {
        self.daily_loss_usd.unwrap_or(DEFAULT_DAILY_LOSS_USD)
    }

    /// The most gross the caps allow: `max_positions` legs of `notional_per_symbol`.
    fn max_gross_usd(&self) -> Decimal {
        self.notional_per_symbol.saturating_mul(Decimal::from(self.max_positions))
    }
}

const fn upto(max: Decimal) -> Range {
    Range { min: None, max: Some(max) }
}

const fn tenths(n: u32) -> Decimal {
    Decimal::from_parts(n, 0, 0, false, 1)
}

const fn hundredths(n: u32) -> Decimal {
    Decimal::from_parts(n, 0, 0, false, 2)
}

/// The tunables as declared params. Hard is the range [`SimpleFundingConfig::validate`] enforces
/// (a `None` side is its "positive" check). The assumed hold's guarded range ends at its default:
/// a longer one lowers the entry hurdle, and only the owner signs it (security agent-surface-M3).
pub const PARAM_SPECS: &[ParamSpec] = &[
    ParamSpec::decimal(
        "notional_per_symbol",
        whole(50),
        Range::RUNTIME,
        Range::RUNTIME,
        "Target notional per leg (USD); every leg is this size.",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Up),
    ParamSpec::int(
        "top_k",
        2,
        Range::whole(1, 25),
        Range::whole(1, 25),
        "Legs per side: short the top positive signals, long the most negative.",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Up),
    ParamSpec::int(
        "max_positions",
        4,
        Range::whole(1, 50),
        Range::whole(1, 50),
        "Maximum concurrently open symbols.",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Up),
    ParamSpec::decimal(
        "max_net_pct",
        hundredths(15),
        Range::new(Decimal::ZERO, whole(1)),
        Range::new(Decimal::ZERO, whole(1)),
        "The long and short books stay within this share of gross of each other.",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Up),
    ParamSpec::decimal(
        "max_oi_pct",
        hundredths(5),
        upto(tenths(5)),
        upto(tenths(5)),
        "Cap on a leg as a fraction of the market's open interest.",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Up),
    ParamSpec::decimal(
        "max_leverage",
        whole(1),
        upto(whole(5)),
        upto(whole(5)),
        "Cap on gross notional over account equity.",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Up),
    ParamSpec::int(
        "signal_window_hours",
        24,
        Range::whole(1, 480),
        Range::whole(1, 480),
        "Settled-funding window the signal averages.",
    )
    .tier(Tier::Structural),
    ParamSpec::int(
        "expected_hold_hours",
        72,
        Range::whole(1, 72),
        Range::whole(1, 720),
        "Hold time assumed when amortising the round trip against funding.",
    )
    .tier(Tier::Structural)
    .loosening(Loosening::Up),
    ParamSpec::decimal(
        "slippage_bps",
        whole(10),
        upto(whole(100)),
        upto(whole(100)),
        "How far past the touch an IOC order may fill (bps).",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Up),
    ParamSpec::decimal(
        "min_depth_usd",
        whole(5000),
        Range::RUNTIME,
        Range::RUNTIME,
        "Books thinner than this (USD, the thinner side) are not entered.",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Down),
    ParamSpec::decimal(
        "daily_loss_usd",
        DEFAULT_DAILY_LOSS_USD,
        Range::RUNTIME,
        Range::RUNTIME,
        "Equity drop since the UTC day start (USD) that flattens everything and halts. A live \
         run must set it explicitly (positive, at most max_positions x notional_per_symbol).",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Up),
    ParamSpec::int(
        "stale_secs",
        30,
        Range::whole(5, 600),
        Range::whole(5, 600),
        "No entries on a market whose mark or book is older than this.",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Up),
    ParamSpec::bool(
        "adopt_existing_positions",
        false,
        "Manage positions the universe already holds that this bot did not open; off, a live \
         start with one is refused.",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Up),
];

fn d_notional() -> Decimal {
    Decimal::from(50)
}

fn d_top_k() -> usize {
    2
}

fn d_max_positions() -> usize {
    4
}

fn d_max_net_pct() -> Decimal {
    hundredths(15)
}

fn d_oi_pct() -> Decimal {
    hundredths(5)
}

fn d_lev() -> Decimal {
    Decimal::ONE
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

fn d_depth() -> Decimal {
    Decimal::from(5000)
}

fn d_stale() -> u64 {
    30
}

impl ValidateConfig for SimpleFundingConfig {
    fn validate(&self) -> Result<(), String> {
        if self.notional_per_symbol <= Decimal::ZERO {
            return Err("notional_per_symbol must be positive".into());
        }
        if !(1..=25).contains(&self.top_k) {
            return Err("top_k must be between 1 and 25".into());
        }
        if !(1..=50).contains(&self.max_positions) {
            return Err("max_positions must be between 1 and 50".into());
        }
        if self.max_net_pct < Decimal::ZERO || self.max_net_pct > Decimal::ONE {
            return Err("max_net_pct must be in [0, 1]".into());
        }
        if self.max_oi_pct <= Decimal::ZERO || self.max_oi_pct > tenths(5) {
            return Err("max_oi_pct must be in (0, 0.5]".into());
        }
        if self.max_leverage <= Decimal::ZERO || self.max_leverage > Decimal::from(5) {
            return Err("max_leverage must be in (0, 5]".into());
        }
        if !(1..=480).contains(&self.signal_window_hours) {
            return Err("signal_window_hours must be between 1 and 480".into());
        }
        if !(1..=720).contains(&self.expected_hold_hours) {
            return Err("expected_hold_hours must be between 1 and 720".into());
        }
        if self.slippage_bps <= Decimal::ZERO || self.slippage_bps > Decimal::from(100) {
            return Err("slippage_bps must be in (0, 100]".into());
        }
        if self.min_depth_usd < Decimal::ZERO {
            return Err("min_depth_usd must not be negative".into());
        }
        if self.daily_loss_usd.is_some_and(|v| v <= Decimal::ZERO) {
            return Err("daily_loss_usd must be positive".into());
        }
        if !(5..=600).contains(&self.stale_secs) {
            return Err("stale_secs must be between 5 and 600".into());
        }
        if let Some(s) = self.exclude.iter().find(|s| self.only.contains(s)) {
            return Err(format!("{s} is in both exclude and only"));
        }
        Ok(())
    }
}

impl SimpleFundingConfig {
    /// What a live run needs beyond [`ValidateConfig::validate`]: an explicit daily loss stop no
    /// larger than the whole book can lose. The build calls it when the document is live (the mode
    /// is the document's; the strategy has none of its own).
    ///
    /// # Errors
    /// No explicit daily loss stop, or one larger than `max_positions x notional_per_symbol`.
    pub fn check_live(&self) -> Result<(), String> {
        // A live run never leans on a default stop: the owner states the loss they accept.
        let Some(loss) = self.daily_loss_usd else {
            return Err("live needs an explicit daily_loss_usd > 0: the daily loss stop has no \
                        live default"
                .into());
        };
        if loss > self.max_gross_usd() {
            return Err(format!(
                "daily_loss_usd {loss} is more than the whole book can lose \
                 (max_positions x notional_per_symbol = {}): that is no stop",
                self.max_gross_usd()
            ));
        }
        Ok(())
    }
}

/// Every `[strategies.simple-funding]` setting: the tunables ([`PARAM_SPECS`])
/// and the symbol lists.
#[must_use]
pub fn settings() -> Vec<Setting> {
    let mut settings: Vec<Setting> = PARAM_SPECS.iter().copied().map(Setting::of).collect();
    settings.push(Setting::of(
        ParamSpec::list("exclude", &[], "Symbols never traded.")
            .set()
            .tier(Tier::Limit)
            .loosening(Loosening::Up),
    ));
    settings.push(Setting::of(
        ParamSpec::list("only", &[], "When set, the only symbols traded.")
            .set()
            .tier(Tier::Limit)
            .loosening(Loosening::Up),
    ));
    settings
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
