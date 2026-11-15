//! What a component says it is, before anything runs: its name, its version and every setting it
//! takes, in the shape bb-core's strategy descriptors use (`Setting`, `ParamSpec`), so the plugin
//! store can read them from the bytes.

use rust_decimal::Decimal;

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

/// How a setting is typed (bb-core's `ParamKind`).
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

/// Who may change a setting, and how it applies (bb-core's `Tier`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Tune,
    Structural,
    Limit,
    Guard,
}

/// A range; an open side is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Range {
    pub min: Option<Decimal>,
    pub max: Option<Decimal>,
}

impl Range {
    #[must_use]
    pub fn new(min: Decimal, max: Decimal) -> Self {
        Self { min: Some(min), max: Some(max) }
    }
}

/// Which way a change loosens a limit or guard (bb-core's `Loosening`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loosening {
    None,
    Up,
    Down,
}

/// One setting (bb-core's `Setting` over a `ParamSpec`).
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
