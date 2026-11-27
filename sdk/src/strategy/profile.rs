//! What a built-in strategy is beyond its code (native only; `describe` in the WIT world stays as
//! it is): the owner's page text, whether it has paper and a daily loss stop, the exchanges it
//! needs, and its presets. The bot's own strategy descriptor is made of these.

use std::collections::BTreeMap;

use rust_decimal::{Decimal, RoundingStrategy};
use serde::Serialize;

use crate::Setting;

/// A built-in's profile ([`Describe::profile`](crate::Describe::profile)).
#[derive(Debug, Clone)]
pub struct Profile {
    pub about: About,
    /// Every setting, in its full form (the descriptor's `settings` are their wire form).
    pub settings: Vec<Setting>,
    /// It has a paper mode. One without runs live whenever it runs: a switch to it must carry the
    /// go-live capability.
    pub supports_paper: bool,
    /// It stops itself at a daily loss. One without is only ever started or switched to by the
    /// owner's signature: no unlock covers it.
    pub has_loss_stop: bool,
    pub exchanges: Vec<ExchangeRole>,
    pub presets: Vec<Preset>,
}

/// A venue the strategy trades on or reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ExchangeRole {
    /// `venue` (where it trades), `hedge`, `reference`.
    pub role: &'static str,
    /// The setting that names the exchange; `None`: fixed to `default`.
    pub setting: Option<&'static str>,
    pub default: &'static str,
    /// It must have a broker (not a price reference only).
    pub tradable: bool,
    /// It is used whatever the settings (a reference can be optional).
    pub required: bool,
}

/// How much a strategy can lose on a bad day, in the owner's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    LowMedium,
    Medium,
    MediumHigh,
    High,
}

/// A strategy's risk: its level and one sentence on what goes wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Risk {
    pub level: RiskLevel,
    pub text: &'static str,
}

/// A setting the owner's page shows first, in plain words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Featured {
    /// The setting's path in the profile's `settings`.
    pub path: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    /// Its part of the folded settings line, `{}` the value (`every {} s`).
    pub fold: &'static str,
    /// A fraction the page shows and takes as a percent (0.02 is 2).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub percent: bool,
}

impl Featured {
    #[must_use]
    pub const fn new(
        path: &'static str,
        label: &'static str,
        hint: &'static str,
        fold: &'static str,
    ) -> Self {
        Self { path, label, hint, fold, percent: false }
    }

    /// Shown and taken as a percent.
    #[must_use]
    pub const fn percent(mut self) -> Self {
        self.percent = true;
        self
    }
}

/// What the owner's page says about a strategy, with no jargon: its name, one line for the
/// picker, what it does, its risk, and the settings shown first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct About {
    pub title: &'static str,
    pub tagline: &'static str,
    pub blurb: &'static str,
    pub risk: Risk,
    pub featured: Vec<Featured>,
}

/// The dollars a preset takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Amounts {
    pub min: u32,
    pub default: u32,
    pub max: u32,
}

/// A market's price and the smallest order step, which a preset's sizes are rounded to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetMarket {
    pub price: Decimal,
    pub step: Decimal,
}

/// A preset's sizing: every setting for the amount, by path in the strategy's section.
pub type SizeFn = fn(Decimal, Option<PresetMarket>) -> Result<BTreeMap<String, String>, String>;

/// A preset: a market and a full set of settings sized from the amount the owner chooses, so a
/// beginner picks one in plain words. The bot checks the amount against `amounts` (and rounds it
/// down to the cent) before it asks for the settings.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    pub id: &'static str,
    pub title: &'static str,
    /// `None` for a strategy that picks its own markets.
    pub symbol: Option<&'static str>,
    /// What it does, in one line.
    pub what: &'static str,
    /// When it makes and loses money, in one line.
    pub money: &'static str,
    pub amounts: Amounts,
    /// Sizing it needs the market's price and step.
    pub needs_market: bool,
    /// Every setting for the amount, by path in the strategy's section, or why that amount can't be
    /// sized on the market.
    pub settings: SizeFn,
    /// The worst case per day at the amount, in one line.
    pub worst_case: fn(Decimal) -> String,
}

/// `pct` percent of `amount`, down to the cent.
#[must_use]
pub fn pct(amount: Decimal, pct: u32) -> Decimal {
    amount
        .checked_mul(Decimal::from(pct))
        .and_then(|v| v.checked_div(Decimal::ONE_HUNDRED))
        .unwrap_or_default()
        .round_dp_with_strategy(2, RoundingStrategy::ToZero)
}

/// `value` down to a whole number of `step`s (zero for a step of zero).
#[must_use]
pub fn floor_to(value: Decimal, step: Decimal) -> Decimal {
    value
        .checked_div(step)
        .map(|n| n.floor())
        .and_then(|n| n.checked_mul(step))
        .unwrap_or_default()
        .normalize()
}

/// Dollars as the owner reads them: `$250`, `$250.50`.
#[must_use]
pub fn money(v: Decimal) -> String {
    if v.fract().is_zero() { format!("${}", v.trunc()) } else { format!("${v:.2}") }
}

/// A setting's value as text.
#[must_use]
pub fn text(v: Decimal) -> String {
    v.normalize().to_string()
}

/// The market a preset titled `title` is sized on, when both its price and step are positive.
///
/// # Errors
/// No market, or one with no usable price, in words.
pub fn usable(title: &str, market: Option<PresetMarket>) -> Result<PresetMarket, String> {
    let m = market.ok_or_else(|| format!("{title} needs the market's price"))?;
    if m.price <= Decimal::ZERO || m.step <= Decimal::ZERO {
        return Err(format!("{title}: no usable price for the market"));
    }
    Ok(m)
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
