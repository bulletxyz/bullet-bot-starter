//! What a component says it is before anything runs (name, version, settings), so the plugin store can read it
//! from the bytes.

use rust_decimal::Decimal;

pub use crate::param_spec::{Loosening, Range, Tier};

/// How a strategy describes itself; `describe` must not depend on settings (it runs before
/// `init`).
/// The name and version are constants because `export_strategy!` also writes them into the
/// component's `bb-strategy-meta` custom section, which the host reads from the file before
/// running any of its code: that is what the owner's approval names. The component's `describe`
/// reports the same two constants, so the section and the code cannot disagree.
pub trait Describe {
    /// The strategy's name (lowercase letters, digits, `-` and `_`).
    const NAME: &'static str;
    /// The strategy's own version (semver), usually `env!("CARGO_PKG_VERSION")`.
    const VERSION: &'static str;

    fn describe() -> Descriptor;

    /// A built-in's profile (native only, never in the WIT world): its owner page, its paper and
    /// loss stop, its exchanges and presets. `None` (the default, every plugin): the bot's page
    /// falls back to the summary.
    fn profile() -> Option<crate::Profile> {
        None
    }

    /// What the host wires for it (native only; a plugin gets the default): see [`Needs`]. The
    /// candles it warms up on depend on its settings, so they are the built strategy's
    /// ([`StrategyAdapter::history`](crate::StrategyAdapter::history)), not this.
    fn needs() -> Needs {
        Needs::default()
    }
}

/// Ticks in a row the venue's position must disagree with the fills before the host takes the
/// venue's over, unless a strategy says otherwise.
pub const DEFAULT_RESYNC_TICKS: u32 = 2;

/// A feed the host delivers beyond the strategy's own book, fills and orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    /// The venue's mark price ([`Event::Mark`](crate::Event::Mark)).
    Mark,
    /// The reference price from another venue ([`Event::Reference`](crate::Event::Reference)).
    Reference,
}

/// What the host wires for a strategy ([`Describe::needs`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Needs {
    pub feeds: Vec<Feed>,
    /// Ticks in a row the venue's position must disagree with the fills before the host takes the
    /// venue's over (new orders stop while a gap is pending); `None`: off.
    pub resync: Option<u32>,
    /// The wind-down closes the position until the venue reads flat.
    pub flatten_on_wind_down: bool,
}

impl Default for Needs {
    /// No extra feed, the host's resync on, no flatten at wind-down.
    fn default() -> Self {
        Self { feeds: Vec::new(), resync: Some(DEFAULT_RESYNC_TICKS), flatten_on_wind_down: false }
    }
}

/// A component's name, version and settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Descriptor {
    pub summary: String,
    pub settings: Vec<SettingSpec>,
    /// The JSON schema of the settings `init` takes (`schemars::schema_for!(TheSettings)`).
    pub settings_schema: schemars::Schema,
    /// The legs it needs besides the one it trades (another venue account, e.g. a hedge): the
    /// host refuses to start it unless each is configured. Empty for a strategy on one symbol.
    pub legs: Vec<LegNeed>,
}

/// A leg a strategy needs: its name for it (what [`Intent::Leg`](crate::Intent) carries) and the
/// venue it must be on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegNeed {
    pub id: String,
    pub venue: String,
}

impl LegNeed {
    #[must_use]
    pub fn new(id: &str, venue: &str) -> Self {
        Self { id: id.into(), venue: venue.into() }
    }
}

/// How a setting is typed (the wire form of [`ParamKind`](crate::param_spec::ParamKind)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingKind {
    Decimal,
    Int,
    /// 0 (off) or 1 (on).
    Flag,
    /// One of these; the default is the index of one.
    Choice(Vec<String>),
    /// Free text, with its default.
    Text(String),
    /// A list of strings, with its default.
    Words(Vec<String>),
}

/// One setting (the wire form of a [`Setting`](crate::Setting) over a
/// [`ParamSpec`](crate::param_spec::ParamSpec)).
#[derive(Debug, Clone, PartialEq)]
pub struct SettingSpec {
    /// `a.b` is `b` in table `a`.
    pub path: String,
    pub kind: SettingKind,
    pub default: Decimal,
    /// The range used by default.
    pub guarded: Range,
    /// The absolute limit; covers `guarded`.
    pub hard: Range,
    pub help: String,
    pub tier: Tier,
    /// For a `Limit` or `Guard`: the direction that loosens it, so a raise or a cut needs the
    /// owner as the descriptor says.
    pub loosening: Loosening,
    /// The settings must give it.
    pub required: bool,
}

impl SettingSpec {
    /// A decimal setting at `path` with one range for both bounds.
    #[must_use]
    pub fn decimal(path: &str, default: Decimal, range: Range, help: &str) -> Self {
        Self {
            path: path.into(),
            kind: SettingKind::Decimal,
            default,
            guarded: range,
            hard: range,
            help: help.into(),
            tier: Tier::Tune,
            loosening: Loosening::None,
            required: false,
        }
    }

    /// An integer setting at `path`.
    #[must_use]
    pub fn int(path: &str, default: u32, range: Range, help: &str) -> Self {
        Self { kind: SettingKind::Int, ..Self::decimal(path, Decimal::from(default), range, help) }
    }

    /// A flag setting at `path`: on or off, with its default.
    #[must_use]
    pub fn flag(path: &str, default: bool, help: &str) -> Self {
        let range = Range::new(Decimal::ZERO, Decimal::ONE);
        Self { kind: SettingKind::Flag, ..Self::int(path, u32::from(default), range, help) }
    }

    /// A choice setting at `path`: one of `choices`, `default` the index of one.
    #[must_use]
    pub fn choice(path: &str, choices: &[&str], default: u32, help: &str) -> Self {
        let last = u32::try_from(choices.len().saturating_sub(1)).unwrap_or(u32::MAX);
        let range = Range::new(Decimal::ZERO, Decimal::from(last));
        let kind = SettingKind::Choice(choices.iter().map(|c| (*c).to_string()).collect());
        Self { kind, ..Self::int(path, default, range, help) }
    }

    #[must_use]
    pub fn tier(mut self, tier: Tier) -> Self {
        self.tier = tier;
        self
    }

    #[must_use]
    pub fn loosening(mut self, loosening: Loosening) -> Self {
        self.loosening = loosening;
        self
    }

    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
}

/// A built-in's [`Setting`](crate::Setting) as the wire form a [`Descriptor`] lists.
impl From<&crate::Setting> for SettingSpec {
    fn from(s: &crate::Setting) -> Self {
        use crate::param_spec::ParamKind;
        let words = |w: &[&str]| w.iter().map(|s| (*s).to_string()).collect();
        let p = &s.spec;
        Self {
            path: s.path.clone(),
            kind: match p.kind {
                ParamKind::Decimal => SettingKind::Decimal,
                ParamKind::Int => SettingKind::Int,
                ParamKind::Bool => SettingKind::Flag,
                ParamKind::Enum { choices } => SettingKind::Choice(words(choices)),
                ParamKind::Text { default } => SettingKind::Text(default.into()),
                ParamKind::List { default } => SettingKind::Words(words(default)),
            },
            default: p.default,
            guarded: p.guarded,
            hard: p.hard,
            help: p.help.into(),
            tier: p.tier,
            loosening: p.loosening,
            required: s.required,
        }
    }
}

#[cfg(test)]
#[path = "describe_tests.rs"]
mod tests;
