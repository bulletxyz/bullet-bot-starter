//! The params a running market maker quotes with ([`MmParams`]) and a live change of them: each
//! value checked against its declared range, the guarded one unless the config store authorized
//! the change (`BoundsMode::Extended`, never past the hard one).

use std::collections::BTreeMap;

use bb_strategy_sdk::SettingChanges;
use bb_strategy_sdk::param_spec::{BoundsMode, ParamKind, ParamSpec, Range, Violation};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::settings::{BUILTIN_SPECS, MarketMakerSettings, PARAMS};

/// Where the fair value the quotes are centred on comes from.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FairValue {
    /// The Binance USD-M perp mid plus an EMA of the basis to the Bullet mid.
    #[default]
    Binance,
    /// The Bullet mid.
    Bullet,
}

impl FairValue {
    /// The venue the market maker trades on: its own mid is the plain fair value.
    pub const TRADED: Self = Self::Bullet;

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Binance => "binance",
            Self::Bullet => "bullet",
        }
    }

    /// The venue's name as a person reads it.
    #[must_use]
    pub fn venue(self) -> &'static str {
        match self {
            Self::Binance => "Binance",
            Self::Bullet => "Bullet",
        }
    }

    /// Quotes follow another venue's mid plus the averaged gap to the traded venue's.
    #[must_use]
    pub fn is_external(self) -> bool {
        self != Self::TRADED
    }
}

impl std::fmt::Display for FairValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for FairValue {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_ascii_lowercase();
        [Self::Binance, Self::Bullet]
            .into_iter()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| format!("fair_value={s} must be one of binance, bullet"))
    }
}

/// The params a live change may set. The JSON is the report's `params`: readers know it.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct MmParams {
    pub bid_spread_bps: Decimal,
    pub ask_spread_bps: Decimal,
    pub order_size: Decimal,
    pub refresh_secs: u64,
    pub refresh_threshold_bps: Decimal,
    pub skew_bps: Decimal,
    pub fair_value: FairValue,
    pub basis_halflife_secs: u64,
    pub reference_stale_secs: u64,
    pub max_divergence_bps: Decimal,
    /// The strategy's own params ([`PARAMS`]), by name; left out of the JSON when empty.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Decimal>,
}

/// One of the strategy's own params as the report lists it: its guarded range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct ExtraParam {
    pub name: &'static str,
    pub default: Decimal,
    pub min: Decimal,
    pub max: Decimal,
    pub help: &'static str,
}

impl From<&ParamSpec> for ExtraParam {
    fn from(s: &ParamSpec) -> Self {
        Self {
            name: s.name,
            default: s.default,
            min: s.guarded.min.unwrap_or(Decimal::MIN),
            max: s.guarded.max.unwrap_or(Decimal::MAX),
            help: s.help,
        }
    }
}

/// `order_size`'s range, known once the venue's step is: the step to half the position cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeBounds {
    pub min: Decimal,
    pub max: Decimal,
}

impl SizeBounds {
    #[must_use]
    pub fn new(step: Option<Decimal>, max_position: Decimal) -> Self {
        let max = max_position.checked_div(Decimal::TWO).unwrap_or_default();
        Self { min: step.unwrap_or_default(), max }
    }
}

fn out_of_bounds(name: &str, value: Decimal, min: Decimal, max: Decimal) -> String {
    format!("{name}={value} outside [{min}, {max}]")
}

fn violation(name: &str, value: Decimal, v: Violation) -> String {
    match v {
        Violation::OutOfBounds { min, max } => out_of_bounds(name, value, min, max),
        Violation::NotInteger => format!("{name}={value} must be an integer"),
        Violation::NotBool => format!("{name}={value} must be one of 0 (off), 1 (on)"),
    }
}

fn spec(name: &str) -> Option<&'static ParamSpec> {
    BUILTIN_SPECS.iter().chain(PARAMS).find(|s| s.name == name)
}

impl MmParams {
    /// The section's params, each of the strategy's own at its `extra` value, else its default.
    #[must_use]
    pub fn from_settings(s: &MarketMakerSettings) -> Self {
        let mut extra = s.extra.clone();
        for p in PARAMS {
            extra.entry(p.name.to_string()).or_insert(p.default);
        }
        Self {
            bid_spread_bps: s.bid_spread_bps,
            ask_spread_bps: s.ask_spread_bps,
            order_size: s.order_size,
            refresh_secs: s.refresh_secs,
            refresh_threshold_bps: s.refresh_threshold_bps,
            skew_bps: s.skew_bps,
            fair_value: s.fair_value,
            basis_halflife_secs: s.basis_halflife_secs,
            reference_stale_secs: s.reference_stale_secs,
            max_divergence_bps: s.max_divergence_bps,
            extra,
        }
    }

    /// The value of the param called `name`, if it exists.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Decimal> {
        Some(match name {
            "bid_spread_bps" => self.bid_spread_bps,
            "ask_spread_bps" => self.ask_spread_bps,
            "order_size" => self.order_size,
            "refresh_secs" => Decimal::from(self.refresh_secs),
            "refresh_threshold_bps" => self.refresh_threshold_bps,
            "skew_bps" => self.skew_bps,
            "basis_halflife_secs" => Decimal::from(self.basis_halflife_secs),
            "reference_stale_secs" => Decimal::from(self.reference_stale_secs),
            "max_divergence_bps" => self.max_divergence_bps,
            _ => return self.extra.get(name).copied(),
        })
    }

    /// The strategy's own param `name`, else its declared default.
    #[must_use]
    pub fn own(&self, name: &str) -> Decimal {
        self.extra.get(name).copied().or_else(|| spec(name).map(|s| s.default)).unwrap_or_default()
    }

    /// Whether the switch called `name` is on (1).
    #[must_use]
    pub fn flag(&self, name: &str) -> bool {
        self.own(name) == Decimal::ONE
    }

    /// Check the param `name` against its range under `mode`; `order_size` against `size` (no
    /// check without one).
    ///
    /// # Errors
    /// The value is out of its range, or not whole where it must be.
    pub fn check(
        &self,
        name: &str,
        mode: BoundsMode,
        size: Option<SizeBounds>,
    ) -> Result<(), String> {
        let (Some(spec), Some(value)) = (spec(name), self.get(name)) else { return Ok(()) };
        if spec.range(mode) == Range::RUNTIME && !matches!(spec.kind, ParamKind::Enum { .. }) {
            let Some(size) = size else { return Ok(()) };
            if value <= Decimal::ZERO || value < size.min || value > size.max {
                return Err(out_of_bounds(name, value, size.min, size.max));
            }
            return Ok(());
        }
        spec.check(value, mode).map_err(|v| violation(name, value, v))
    }

    /// These params with `changes` applied, each changed one checked under `changes.bounds`.
    ///
    /// # Errors
    /// A name that is no param, a value that is not a number (or not a choice for `fair_value`),
    /// not whole where it must be, or out of its range.
    pub fn changed(&self, changes: &SettingChanges, size: SizeBounds) -> Result<Self, String> {
        let mut next = self.clone();
        for (name, text) in &changes.values {
            if name == "fair_value" {
                next.fair_value = text.parse()?;
                continue;
            }
            let value: Decimal =
                text.trim().parse().map_err(|_| format!("{name}={text} must be a number"))?;
            let whole = || {
                value
                    .fract()
                    .is_zero()
                    .then(|| u64::try_from(value.trunc()).ok())
                    .flatten()
                    .ok_or_else(|| format!("{name}={value} must be an integer"))
            };
            match name.as_str() {
                "bid_spread_bps" => next.bid_spread_bps = value,
                "ask_spread_bps" => next.ask_spread_bps = value,
                "order_size" => next.order_size = value,
                "refresh_threshold_bps" => next.refresh_threshold_bps = value,
                "skew_bps" => next.skew_bps = value,
                "max_divergence_bps" => next.max_divergence_bps = value,
                "refresh_secs" => next.refresh_secs = whole()?,
                "basis_halflife_secs" => next.basis_halflife_secs = whole()?,
                "reference_stale_secs" => next.reference_stale_secs = whole()?,
                _ if PARAMS.iter().any(|p| p.name == name) => {
                    next.extra.insert(name.clone(), value);
                }
                _ => return Err(format!("unknown parameter {name}")),
            }
            next.check(name, changes.bounds, Some(size))?;
        }
        Ok(next)
    }

    /// `name old → new` for each param `changes` set, as the reply says it.
    #[must_use]
    pub fn summary(&self, before: &Self, changes: &SettingChanges) -> String {
        let text = |p: &Self, name: &str| match name {
            "fair_value" => p.fair_value.to_string(),
            _ => p.get(name).map_or_else(String::new, |v| v.to_string()),
        };
        let parts: Vec<String> = changes
            .values
            .keys()
            .map(|n| format!("{n} {} → {}", text(before, n), text(self, n)))
            .collect();
        parts.join(", ")
    }
}

#[cfg(test)]
#[path = "params_tests.rs"]
mod tests;
