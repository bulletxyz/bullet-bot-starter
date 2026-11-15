//! Declared parameters: what a strategy, the market-maker runtime or a guard
//! lets be tuned, with two sets of bounds.
//!
//! Every value is a [`Decimal`]: an `Int` is a whole number, a `Bool` is 0 or
//! 1, an `Enum` is the index of a choice. A `Text` is the exception: a free
//! string (a venue name, a list of dexes) that carries its own default and has
//! no bounds. `guarded` is the range used by
//! default ([`BoundsMode::Guarded`]); `hard` is the engineering limit no
//! profile may pass ([`BoundsMode::Extended`] allows up to it). A side left
//! `None` is not fixed here: the owner resolves it at runtime (e.g. an order
//! size between the venue step and half the max position).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::text_format::TextFormat;

/// What kind of value a parameter takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParamKind {
    Decimal,
    Int,
    /// 0 (off) or 1 (on).
    Bool,
    /// The index of one of `choices`.
    Enum {
        choices: &'static [&'static str],
    },
    /// A free string; not a [`Decimal`], so `default` and the bounds don't apply.
    Text {
        default: &'static str,
    },
    /// A list of strings (symbols, dex names); like [`ParamKind::Text`], unbounded.
    List {
        default: &'static [&'static str],
    },
}

/// An inclusive range; a `None` side is resolved at runtime by the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Range {
    pub min: Option<Decimal>,
    pub max: Option<Decimal>,
}

impl Range {
    /// Both sides resolved at runtime.
    pub const RUNTIME: Self = Self { min: None, max: None };

    #[must_use]
    pub const fn new(min: Decimal, max: Decimal) -> Self {
        Self { min: Some(min), max: Some(max) }
    }

    /// `min..=max` in whole numbers.
    #[must_use]
    pub const fn whole(min: u32, max: u32) -> Self {
        Self::new(whole(min), whole(max))
    }

    #[must_use]
    pub fn contains(&self, value: Decimal) -> bool {
        self.min.is_none_or(|m| value >= m) && self.max.is_none_or(|m| value <= m)
    }

    /// Every value `inner` allows, this one does too.
    #[must_use]
    pub fn covers(&self, inner: &Self) -> bool {
        let lo = match (self.min, inner.min) {
            (None, _) => true,
            (Some(a), Some(b)) => a <= b,
            (Some(_), None) => false,
        };
        let hi = match (self.max, inner.max) {
            (None, _) => true,
            (Some(a), Some(b)) => a >= b,
            (Some(_), None) => false,
        };
        lo && hi
    }
}

/// `n` as a [`Decimal`], in const contexts.
#[must_use]
pub const fn whole(n: u32) -> Decimal {
    Decimal::from_parts(n, 0, 0, false, 0)
}

/// Which direction of change takes more risk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Loosening {
    Up,
    Down,
    None,
}

/// How a change to the parameter is applied and who may make it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Live, within its bounds.
    Tune,
    /// Rebuilds the strategy (symbol, strategy choice, venue switches).
    Structural,
    /// A size or loss limit.
    Limit,
    /// A guard threshold.
    Guard,
}

/// Which bounds a change is checked against: the guarded range (default, and
/// always for the agent and Telegram) or the hard one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundsMode {
    #[default]
    Guarded,
    Extended,
}

/// One declared parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ParamSpec {
    /// Lower-case letters, digits and `_`.
    pub name: &'static str,
    pub kind: ParamKind,
    pub default: Decimal,
    /// The range used by default ([`BoundsMode::Guarded`]).
    pub guarded: Range,
    /// The absolute limit; covers `guarded`.
    pub hard: Range,
    pub loosening: Loosening,
    /// What it does, for the agent and the owner.
    pub help: &'static str,
    pub tier: Tier,
    /// The format a text value (or each list item) must have.
    pub format: TextFormat,
    /// Set only where the bot starts, never by a config change: the refusal's reason, in words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed: Option<&'static str>,
    /// A list that names each item once: a repeated item is refused. Other lists keep repeats.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub set: bool,
}

/// Why a value is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Violation {
    NotInteger,
    NotBool,
    /// Outside the range of the mode checked; an unfixed side is reported as
    /// `Decimal::MIN` / `Decimal::MAX`.
    OutOfBounds {
        min: Decimal,
        max: Decimal,
    },
}

impl ParamSpec {
    const fn of(
        name: &'static str,
        kind: ParamKind,
        default: Decimal,
        guarded: Range,
        hard: Range,
        help: &'static str,
    ) -> Self {
        Self {
            name,
            kind,
            default,
            guarded,
            hard,
            loosening: Loosening::None,
            help,
            tier: Tier::Tune,
            format: TextFormat::Free,
            fixed: None,
            set: false,
        }
    }

    #[must_use]
    pub const fn decimal(
        name: &'static str,
        default: Decimal,
        guarded: Range,
        hard: Range,
        help: &'static str,
    ) -> Self {
        Self::of(name, ParamKind::Decimal, default, guarded, hard, help)
    }

    #[must_use]
    pub const fn int(
        name: &'static str,
        default: u32,
        guarded: Range,
        hard: Range,
        help: &'static str,
    ) -> Self {
        Self::of(name, ParamKind::Int, whole(default), guarded, hard, help)
    }

    #[must_use]
    pub const fn bool(name: &'static str, default: bool, help: &'static str) -> Self {
        let on = Range::whole(0, 1);
        Self::of(name, ParamKind::Bool, whole(if default { 1 } else { 0 }), on, on, help)
    }

    /// `default` is an index into `choices`.
    #[must_use]
    pub const fn choice(
        name: &'static str,
        choices: &'static [&'static str],
        default: u32,
        help: &'static str,
    ) -> Self {
        #[expect(clippy::cast_possible_truncation, reason = "a handful of choices")]
        let last = choices.len().saturating_sub(1) as u32;
        let all = Range::whole(0, last);
        Self::of(name, ParamKind::Enum { choices }, whole(default), all, all, help)
    }

    /// A list-of-strings setting (e.g. symbols); listed, not range-checked.
    #[must_use]
    pub const fn list(
        name: &'static str,
        default: &'static [&'static str],
        help: &'static str,
    ) -> Self {
        let kind = ParamKind::List { default };
        Self::of(name, kind, Decimal::ZERO, Range::RUNTIME, Range::RUNTIME, help)
    }

    /// A string-valued setting (e.g. a venue name); listed, not range-checked.
    #[must_use]
    pub const fn text(name: &'static str, default: &'static str, help: &'static str) -> Self {
        let kind = ParamKind::Text { default };
        Self::of(name, kind, Decimal::ZERO, Range::RUNTIME, Range::RUNTIME, help)
    }

    #[must_use]
    pub const fn format(mut self, format: TextFormat) -> Self {
        self.format = format;
        self
    }

    #[must_use]
    pub const fn tier(mut self, tier: Tier) -> Self {
        self.tier = tier;
        self
    }

    #[must_use]
    pub const fn loosening(mut self, loosening: Loosening) -> Self {
        self.loosening = loosening;
        self
    }

    /// Set only where the bot starts (`why`: how, and why not live); a change to it is refused.
    #[must_use]
    pub const fn fixed(mut self, why: &'static str) -> Self {
        self.fixed = Some(why);
        self
    }

    /// A list that names each item once (symbols, wallets): a repeated item is refused.
    #[must_use]
    pub const fn set(mut self) -> Self {
        self.set = true;
        self
    }

    /// The items of a list value, checked against the format ([`TextFormat::check_list`]), and
    /// none twice when the list is a [set](Self::set).
    ///
    /// # Errors
    /// An item does not fit, there are too many, or one repeats.
    pub fn check_list(&self, items: &[String]) -> Result<(), String> {
        self.format.check_list(items, self.set)
    }

    /// The range `mode` allows.
    #[must_use]
    pub const fn range(&self, mode: BoundsMode) -> Range {
        match mode {
            BoundsMode::Guarded => self.guarded,
            BoundsMode::Extended => self.hard,
        }
    }

    /// Whether `value` is allowed under `mode`.
    ///
    /// # Errors
    /// The value is not of the parameter's kind or outside the mode's range.
    pub fn check(&self, value: Decimal, mode: BoundsMode) -> Result<(), Violation> {
        match self.kind {
            ParamKind::Bool if value != Decimal::ZERO && value != Decimal::ONE => {
                return Err(Violation::NotBool);
            }
            ParamKind::Int | ParamKind::Enum { .. } if !value.fract().is_zero() => {
                return Err(Violation::NotInteger);
            }
            ParamKind::Decimal
            | ParamKind::Bool
            | ParamKind::Int
            | ParamKind::Enum { .. }
            | ParamKind::Text { .. }
            | ParamKind::List { .. } => {}
        }
        let range = self.range(mode);
        if range.contains(value) {
            Ok(())
        } else {
            Err(Violation::OutOfBounds {
                min: range.min.unwrap_or(Decimal::MIN),
                max: range.max.unwrap_or(Decimal::MAX),
            })
        }
    }

    /// Whether moving from `from` to `to` goes in the riskier direction.
    #[must_use]
    pub fn loosens(&self, from: Decimal, to: Decimal) -> bool {
        match self.loosening {
            Loosening::Up => to > from,
            Loosening::Down => to < from,
            Loosening::None => false,
        }
    }

    /// A consistent declaration: `hard` covers `guarded`, the default is of
    /// the kind and inside `guarded`, a bool or enum stays within its values.
    ///
    /// # Errors
    /// What is wrong with it.
    pub fn check_declaration(&self) -> Result<(), String> {
        if !self.hard.covers(&self.guarded) {
            return Err("needs hard bounds that cover the guarded ones".into());
        }
        let values = match self.kind {
            ParamKind::Bool => Some(Range::whole(0, 1)),
            ParamKind::Enum { choices: [] } => {
                return Err("needs at least one choice".into());
            }
            ParamKind::Enum { choices } => {
                let last = u32::try_from(choices.len().saturating_sub(1)).unwrap_or(u32::MAX);
                Some(Range::whole(0, last))
            }
            ParamKind::Decimal
            | ParamKind::Int
            | ParamKind::Text { .. }
            | ParamKind::List { .. } => None,
        };
        if values.is_some_and(|v| !v.covers(&self.hard)) {
            return Err("has bounds beyond its values".into());
        }
        if let (Some(min), Some(max)) = (self.guarded.min, self.guarded.max)
            && min > max
        {
            return Err(format!("needs min <= max, got {min} {max}"));
        }
        match self.check(self.default, BoundsMode::Guarded) {
            Ok(()) => Ok(()),
            Err(Violation::OutOfBounds { min, max }) => Err(format!(
                "needs min <= default <= max, got {} {} {}",
                self.guarded.min.unwrap_or(min),
                self.default,
                self.guarded.max.unwrap_or(max)
            )),
            Err(_) => Err(format!("default {} is not a {:?} value", self.default, self.kind)),
        }
    }
}

#[cfg(test)]
#[path = "param_spec_tests.rs"]
mod tests;
