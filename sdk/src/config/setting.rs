//! A strategy's settings as data: each at its path in the strategy's config section, over a
//! [`ParamSpec`]. A built-in strategy crate lists its own and the bot reads them (the owner's page,
//! the config store, `bb setup`); [`ValidateConfig`] is the check its config makes after parsing.

use serde::Serialize;
use serde_json::Value;

use crate::param_spec::{ParamKind, ParamSpec};

/// Semantic validation for a parsed config struct.
///
/// TOML/serde parsing answers "is the shape right?". `ValidateConfig` answers "do these values
/// make sense together?".
pub trait ValidateConfig {
    /// # Errors
    /// What is wrong with the values, in plain words.
    fn validate(&self) -> Result<(), String>;
}

/// One setting at `path` in the strategy's config section (`a.b` is `b` in
/// table `a`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Setting {
    pub path: String,
    pub spec: ParamSpec,
    /// The section must give it: no default applies.
    pub required: bool,
    /// It has no safe default though the config loads without it (a size, a loss stop):
    /// `bb setup` asks it. See [`Setting::no_safe_default`].
    #[serde(skip)]
    pub ask: bool,
    /// The host limit this setting sets (a built-in's only; never in the JSON): the host holds the
    /// path to the smaller of its value (its default when unset) and the document's limit.
    #[serde(skip)]
    pub caps: Option<HostLimit>,
}

/// A limit the host holds every strategy's order path to, which a setting may set
/// ([`Setting::caps`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostLimit {
    MaxPosition,
    DailyLossUsd,
}

impl Setting {
    /// `spec` at the top level, optional.
    #[must_use]
    pub fn of(spec: ParamSpec) -> Self {
        Self { path: spec.name.to_string(), spec, required: false, ask: false, caps: None }
    }

    /// `spec` inside table `table`.
    #[must_use]
    pub fn under(table: &str, spec: ParamSpec) -> Self {
        Self {
            path: format!("{table}.{}", spec.name),
            spec,
            required: false,
            ask: false,
            caps: None,
        }
    }

    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// It sets the host limit `limit` (see [`Setting::caps`]).
    #[must_use]
    pub fn caps(mut self, limit: HostLimit) -> Self {
        self.caps = Some(limit);
        self
    }

    /// Mark it as having no safe default (see [`Setting::ask`]).
    #[must_use]
    pub fn ask(mut self) -> Self {
        self.ask = true;
        self
    }

    /// A wizard must ask it (showing its example as the suggestion), never fill it in: a
    /// required hard limit (a position cap, a price bound) or a setting marked
    /// [`Setting::ask`]. A required setting of another tier starts at its default.
    #[must_use]
    pub fn no_safe_default(&self) -> bool {
        self.ask || (self.required && self.spec.tier == crate::param_spec::Tier::Limit)
    }

    /// The top-level key it lives under.
    #[must_use]
    pub fn key(&self) -> &str {
        self.path.split('.').next().unwrap_or(&self.path)
    }

    /// The default (or, for a required setting, an example) as the JSON a
    /// config section holds.
    #[must_use]
    pub fn default_json(&self) -> Value {
        let d = self.spec.default;
        match self.spec.kind {
            ParamKind::Decimal => Value::String(d.normalize().to_string()),
            ParamKind::Int => serde_json::to_value(d.trunc().mantissa()).unwrap_or(Value::Null),
            ParamKind::Bool => Value::Bool(!d.is_zero()),
            ParamKind::Enum { choices } => {
                let i = usize::try_from(d.trunc().mantissa()).unwrap_or(0);
                Value::String(choices.get(i).copied().unwrap_or_default().to_string())
            }
            ParamKind::Text { default } => Value::String(default.to_string()),
            ParamKind::List { default } => {
                Value::Array(default.iter().map(|s| Value::String((*s).to_string())).collect())
            }
        }
    }
}

#[cfg(test)]
#[path = "setting_tests.rs"]
mod tests;
