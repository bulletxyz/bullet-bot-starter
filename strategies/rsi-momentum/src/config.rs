//! `[strategies.rsi-momentum]`: the knobs and their declared settings.

use bb_strategy_sdk::param_spec::{Loosening, ParamSpec, Range, Tier, whole};
use bb_strategy_sdk::{CandleInterval, Setting, ValidateConfig};
use rust_decimal::Decimal;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

/// User-facing knobs for RSI momentum on one Bullet market.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RsiMomentumConfig {
    /// The Bullet market traded.
    pub symbol: String,
    /// Candle length.
    #[serde(default = "d_interval", deserialize_with = "by_name")]
    #[schemars(with = "String")]
    pub interval: CandleInterval,
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
    /// Hard cap on the position, base units. No default: the owner sets it.
    pub max_position: Decimal,
    /// Close when the price moves this fraction against the entry (checked on every mark).
    #[serde(default = "d_stop")]
    pub stop_loss_pct: Decimal,
    /// Equity drop since the UTC day start (USD) that flattens and halts. A live run must set it;
    /// a paper run without one uses [`DEFAULT_DAILY_LOSS_USD`].
    #[serde(default)]
    pub daily_loss_usd: Option<Decimal>,
    /// Closed candles after an exit before a new entry.
    #[serde(default = "d_cooldown")]
    pub cooldown_candles: u32,
    /// How far past the touch an IOC may fill (bps).
    #[serde(default = "d_slippage")]
    pub slippage_bps: Decimal,
    /// No entries on a mark older than this.
    #[serde(default = "d_stale")]
    pub stale_secs: u64,
}

/// The loss stop a paper run assumes when none is set. A live run never gets a default.
pub const DEFAULT_DAILY_LOSS_USD: Decimal = Decimal::from_parts(5, 0, 0, false, 0);

impl RsiMomentumConfig {
    /// What a live run needs beyond [`Self::validate`]: an explicit `daily_loss_usd`, never more
    /// than the whole position. The build calls it when the document is live (the mode is the
    /// document's; the strategy has none of its own).
    ///
    /// # Errors
    /// No explicit daily loss stop, or one larger than `notional_usd`.
    pub fn check_live(&self) -> Result<(), String> {
        // A live run never leans on a default stop: the owner states the loss they accept.
        let Some(loss) = self.daily_loss_usd else {
            return Err("live needs an explicit daily_loss_usd > 0: the daily loss stop has no \
                        live default"
                .into());
        };
        if loss > self.notional_usd {
            return Err(format!(
                "daily_loss_usd {loss} is more than the whole position (notional_usd = {}): that \
                 is no stop",
                self.notional_usd
            ));
        }
        Ok(())
    }

    /// The daily loss stop in force: the configured one, else the paper default.
    #[must_use]
    pub fn loss_stop_usd(&self) -> Decimal {
        self.daily_loss_usd.unwrap_or(DEFAULT_DAILY_LOSS_USD)
    }

    #[must_use]
    pub fn params(&self) -> crate::decide::Params {
        crate::decide::Params {
            long_above: self.long_above,
            short_below: self.short_below,
            exit_band: self.exit_band,
            allow_short: self.allow_short,
        }
    }
}

const fn hundredths(n: u32) -> Decimal {
    Decimal::from_parts(n, 0, 0, false, 2)
}

const RSI_RANGE: Range = Range { min: Some(Decimal::ZERO), max: Some(Decimal::ONE_HUNDRED) };

/// The tunables as declared params. Hard is the range [`RsiMomentumConfig::validate`] enforces (a
/// `None` side is its "positive" check). The bands' guarded ranges are narrower: past them only
/// the owner signs (security agent-surface-M3), and an entry band the owner set is loosened only
/// with their signature. The exit band declares no way: it loosens down for a long and up
/// for a short.
pub const PARAM_SPECS: &[ParamSpec] = &[
    ParamSpec::choice("interval", &CandleInterval::NAMES, 2, "Candle length.")
        .tier(Tier::Structural),
    ParamSpec::int(
        "rsi_period",
        14,
        Range::whole(2, 100),
        Range::whole(2, 100),
        "RSI lookback in candles (Wilder smoothing).",
    )
    .tier(Tier::Structural),
    ParamSpec::decimal(
        "long_above",
        whole(60),
        Range::whole(55, 90),
        RSI_RANGE,
        "Go long when the RSI crosses above this.",
    )
    .loosening(Loosening::Down),
    ParamSpec::decimal(
        "short_below",
        whole(40),
        Range::whole(10, 45),
        RSI_RANGE,
        "Go short (with allow_short) when the RSI crosses below this.",
    )
    .loosening(Loosening::Up),
    ParamSpec::decimal(
        "exit_band",
        whole(50),
        Range::whole(40, 60),
        RSI_RANGE,
        "A long exits when the RSI falls below this, a short when it rises above it.",
    ),
    ParamSpec::bool("allow_short", false, "Take short entries too; off is long only.")
        .tier(Tier::Limit)
        .loosening(Loosening::Up),
    ParamSpec::decimal(
        "notional_usd",
        whole(50),
        Range::RUNTIME,
        Range::RUNTIME,
        "Entry size, USD at the mark.",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Up),
    ParamSpec::decimal(
        "stop_loss_pct",
        hundredths(2),
        Range { min: None, max: Some(Decimal::from_parts(5, 0, 0, false, 1)) },
        Range { min: None, max: Some(Decimal::from_parts(5, 0, 0, false, 1)) },
        "Close when the price moves this fraction against the entry (always on).",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Up),
    ParamSpec::decimal(
        "daily_loss_usd",
        DEFAULT_DAILY_LOSS_USD,
        Range::RUNTIME,
        Range::RUNTIME,
        "Equity drop since the UTC day start (USD) that flattens and halts. A live run must set \
         it explicitly (positive, at most notional_usd).",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Up),
    ParamSpec::int(
        "cooldown_candles",
        2,
        Range::whole(0, 100),
        Range::whole(0, 100),
        "Closed candles after an exit before a new entry.",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Down),
    ParamSpec::decimal(
        "slippage_bps",
        whole(10),
        Range { min: None, max: Some(Decimal::ONE_HUNDRED) },
        Range { min: None, max: Some(Decimal::ONE_HUNDRED) },
        "How far past the touch an IOC may fill (bps).",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Up),
    ParamSpec::int(
        "stale_secs",
        30,
        Range::whole(5, 600),
        Range::whole(5, 600),
        "No entries on a mark older than this.",
    )
    .tier(Tier::Limit)
    .loosening(Loosening::Up),
];

fn d_interval() -> CandleInterval {
    CandleInterval::M15
}

/// The interval by its usual name (`15m`), not the SDK's variant name.
fn by_name<'de, D: Deserializer<'de>>(d: D) -> Result<CandleInterval, D::Error> {
    let name = String::deserialize(d)?;
    CandleInterval::parse(&name)
        .ok_or_else(|| D::Error::unknown_variant(&name, &CandleInterval::NAMES))
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
    hundredths(2)
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

impl ValidateConfig for RsiMomentumConfig {
    fn validate(&self) -> Result<(), String> {
        if self.symbol.trim().is_empty() {
            return Err("symbol must be set".into());
        }
        if !(2..=100).contains(&self.rsi_period) {
            return Err("rsi_period must be between 2 and 100".into());
        }
        for (name, v) in [
            ("long_above", self.long_above),
            ("short_below", self.short_below),
            ("exit_band", self.exit_band),
        ] {
            if v < Decimal::ZERO || v > Decimal::ONE_HUNDRED {
                return Err(format!("{name} must be in [0, 100]"));
            }
        }
        if self.short_below >= self.long_above {
            return Err("short_below must be below long_above".into());
        }
        if self.exit_band < self.short_below || self.exit_band > self.long_above {
            return Err("exit_band must lie between short_below and long_above (else a fresh \
                        entry would exit at once)"
                .into());
        }
        if self.notional_usd <= Decimal::ZERO {
            return Err("notional_usd must be positive".into());
        }
        if self.max_position <= Decimal::ZERO {
            return Err("max_position must be positive".into());
        }
        if self.stop_loss_pct <= Decimal::ZERO || self.stop_loss_pct > Decimal::new(5, 1) {
            return Err("stop_loss_pct must be in (0, 0.5]".into());
        }
        if self.daily_loss_usd.is_some_and(|v| v <= Decimal::ZERO) {
            return Err("daily_loss_usd must be positive".into());
        }
        if self.cooldown_candles > 100 {
            return Err("cooldown_candles must be at most 100".into());
        }
        if self.slippage_bps <= Decimal::ZERO || self.slippage_bps > Decimal::ONE_HUNDRED {
            return Err("slippage_bps must be in (0, 100]".into());
        }
        if !(5..=600).contains(&self.stale_secs) {
            return Err("stale_secs must be between 5 and 600".into());
        }
        Ok(())
    }
}
/// Every `[strategies.rsi-momentum]` setting: the symbol, the position cap and [`PARAM_SPECS`].
#[must_use]
pub fn settings() -> Vec<Setting> {
    let mut settings = vec![
        Setting::of(
            ParamSpec::text("symbol", "BTC-USD", "The Bullet market traded.")
                .tier(Tier::Structural),
        )
        .required(),
        Setting::of(
            ParamSpec::decimal(
                "max_position",
                Decimal::new(1, 3),
                Range::RUNTIME,
                Range::RUNTIME,
                "Hard cap on the position, base units.",
            )
            .tier(Tier::Limit)
            .loosening(Loosening::Up),
        )
        .required(),
    ];
    settings.extend(PARAM_SPECS.iter().copied().map(Setting::of));
    settings
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
