//! The settings: the built-in's `[strategies.simple-funding]`, field for field, with its defaults
//! and checks. Differences, all from how a bot runs a plugin:
//!
//! - The universe is the leg `universe`'s symbols, which the owner sets on the bot
//!   (`legs.universe.symbols`); `only` and `exclude` narrow it as in the built-in.
//! - The built-in refuses a live run without an explicit `daily_loss_usd`; a plugin does not know
//!   whether it is live, so the default 20 USD (the built-in's paper default) always applies and
//!   a value above `max_positions x notional_per_symbol` (no stop at all) is refused always.
//! - The descriptor has no place for the built-in's presets, risk text or "featured" flag.

use bb_strategy_sdk::{Decimal, Loosening, Range, SettingKind, SettingSpec, StrategyError, Tier};
use serde::{Deserialize, Serialize};

/// The loss stop in force when none is set.
pub const DEFAULT_DAILY_LOSS_USD: Decimal = Decimal::from_parts(20, 0, 0, false, 0);

/// The strategy's settings, as JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SimpleFundingSettings {
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
    /// Settled-funding window the signal takes the median of.
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
    /// Equity drop since the UTC day start (USD) that flattens everything and halts for the day.
    #[serde(default)]
    pub daily_loss_usd: Option<Decimal>,
    /// No entries on a market whose mark or book is older than this.
    #[serde(default = "d_stale")]
    pub stale_secs: u64,
    /// Manage (exit, flatten) positions the universe already holds that this strategy did not
    /// open. Off: it does not trade while the account holds such a position.
    #[serde(default)]
    pub adopt_existing_positions: bool,
    /// Symbols never traded.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// If non-empty, the only symbols traded.
    #[serde(default)]
    pub only: Vec<String>,
}

impl SimpleFundingSettings {
    /// The daily loss stop in force.
    #[must_use]
    pub fn loss_stop_usd(&self) -> Decimal {
        self.daily_loss_usd.unwrap_or(DEFAULT_DAILY_LOSS_USD)
    }

    fn max_gross_usd(&self) -> Decimal {
        self.notional_per_symbol.saturating_mul(Decimal::from(self.max_positions))
    }

    /// The built-in's checks, and the loss stop's (see the module doc).
    ///
    /// # Errors
    /// The first setting it cannot run with.
    pub fn validate(&self) -> Result<(), StrategyError> {
        let bad = |why: String| Err(StrategyError(format!("simple-funding: {why}")));
        if self.notional_per_symbol <= Decimal::ZERO {
            return bad("notional_per_symbol must be positive".into());
        }
        if !(1..=25).contains(&self.top_k) {
            return bad("top_k must be between 1 and 25".into());
        }
        if !(1..=50).contains(&self.max_positions) {
            return bad("max_positions must be between 1 and 50".into());
        }
        if self.max_net_pct < Decimal::ZERO || self.max_net_pct > Decimal::ONE {
            return bad("max_net_pct must be in [0, 1]".into());
        }
        if self.max_oi_pct <= Decimal::ZERO || self.max_oi_pct > tenths(5) {
            return bad("max_oi_pct must be in (0, 0.5]".into());
        }
        if self.max_leverage <= Decimal::ZERO || self.max_leverage > Decimal::from(5) {
            return bad("max_leverage must be in (0, 5]".into());
        }
        if !(1..=480).contains(&self.signal_window_hours) {
            return bad("signal_window_hours must be between 1 and 480".into());
        }
        if !(1..=720).contains(&self.expected_hold_hours) {
            return bad("expected_hold_hours must be between 1 and 720".into());
        }
        if self.slippage_bps <= Decimal::ZERO || self.slippage_bps > Decimal::from(100) {
            return bad("slippage_bps must be in (0, 100]".into());
        }
        if self.min_depth_usd < Decimal::ZERO {
            return bad("min_depth_usd must not be negative".into());
        }
        if self.daily_loss_usd.is_some_and(|v| v <= Decimal::ZERO) {
            return bad("daily_loss_usd must be positive".into());
        }
        if let Some(loss) = self.daily_loss_usd.filter(|l| *l > self.max_gross_usd()) {
            return bad(format!(
                "daily_loss_usd {loss} is more than the whole book can lose \
                 (max_positions x notional_per_symbol = {}): that is no stop",
                self.max_gross_usd()
            ));
        }
        if !(5..=600).contains(&self.stale_secs) {
            return bad("stale_secs must be between 5 and 600".into());
        }
        if let Some(s) = self.exclude.iter().find(|s| self.only.contains(s)) {
            return bad(format!("{s} is in both exclude and only"));
        }
        Ok(())
    }
}

fn tenths(n: u32) -> Decimal {
    Decimal::from_parts(n, 0, 0, false, 1)
}

fn hundredths(n: u32) -> Decimal {
    Decimal::from_parts(n, 0, 0, false, 2)
}

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

fn whole(n: i64) -> Decimal {
    Decimal::from(n)
}

fn up_to(max: Decimal) -> Range {
    Range { min: None, max: Some(max) }
}

fn limit(spec: SettingSpec, loosening: Loosening) -> SettingSpec {
    spec.tier(Tier::Limit).loosening(loosening)
}

/// The built-in's tunables with its ranges, tiers and help, in its order.
#[must_use]
#[expect(clippy::too_many_lines, reason = "one table of settings")]
pub fn settings() -> Vec<SettingSpec> {
    let int_range = |lo: i64, hi: i64| Range::new(whole(lo), whole(hi));
    let flag = SettingSpec {
        kind: SettingKind::Flag,
        ..SettingSpec::int(
            "adopt_existing_positions",
            0,
            int_range(0, 1),
            "Manage positions the universe already holds that this strategy did not open; off, it \
             does not trade while the account holds one.",
        )
    };
    let words = |path: &str, help: &str| SettingSpec {
        kind: SettingKind::Words(Vec::new()),
        ..SettingSpec::int(path, 0, Range::default(), help)
    };
    vec![
        limit(
            SettingSpec::decimal(
                "notional_per_symbol",
                whole(50),
                Range::default(),
                "Target notional per leg (USD); every leg is this size.",
            ),
            Loosening::Up,
        ),
        limit(
            SettingSpec::int(
                "top_k",
                2,
                int_range(1, 25),
                "Legs per side: short the top positive signals, long the most negative.",
            ),
            Loosening::Up,
        ),
        limit(
            SettingSpec::int(
                "max_positions",
                4,
                int_range(1, 50),
                "Maximum concurrently open symbols.",
            ),
            Loosening::Up,
        ),
        limit(
            SettingSpec::decimal(
                "max_net_pct",
                hundredths(15),
                Range::new(Decimal::ZERO, whole(1)),
                "The long and short books stay within this share of gross of each other.",
            ),
            Loosening::Up,
        ),
        limit(
            SettingSpec::decimal(
                "max_oi_pct",
                hundredths(5),
                up_to(tenths(5)),
                "Cap on a leg as a fraction of the market's open interest.",
            ),
            Loosening::Up,
        ),
        limit(
            SettingSpec::decimal(
                "max_leverage",
                whole(1),
                up_to(whole(5)),
                "Cap on gross notional over account equity.",
            ),
            Loosening::Up,
        ),
        SettingSpec::int(
            "signal_window_hours",
            24,
            int_range(1, 480),
            "Settled-funding window the signal takes the median of.",
        )
        .tier(Tier::Structural),
        SettingSpec {
            hard: int_range(1, 720),
            ..limit(
                SettingSpec::int(
                    "expected_hold_hours",
                    72,
                    int_range(1, 72),
                    "Hold time assumed when amortising the round trip against funding.",
                ),
                Loosening::Up,
            )
        }
        .tier(Tier::Structural),
        limit(
            SettingSpec::decimal(
                "slippage_bps",
                whole(10),
                up_to(whole(100)),
                "How far past the touch an IOC order may fill (bps).",
            ),
            Loosening::Up,
        ),
        limit(
            SettingSpec::decimal(
                "min_depth_usd",
                whole(5000),
                Range::default(),
                "Books thinner than this (USD, the thinner side) are not entered.",
            ),
            Loosening::Down,
        ),
        limit(
            SettingSpec::decimal(
                "daily_loss_usd",
                DEFAULT_DAILY_LOSS_USD,
                Range::default(),
                "Equity drop since the UTC day start (USD) that flattens everything and halts for \
                 the day (positive, at most max_positions x notional_per_symbol).",
            ),
            Loosening::Up,
        ),
        limit(
            SettingSpec::int(
                "stale_secs",
                30,
                int_range(5, 600),
                "No entries on a market whose mark or book is older than this.",
            ),
            Loosening::Up,
        ),
        limit(flag, Loosening::Up),
        words("exclude", "Symbols never traded."),
        words("only", "If non-empty, the only symbols traded."),
    ]
}
