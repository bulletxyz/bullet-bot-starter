//! Strategy instances: the id every instance's state, keys, journal lines,
//! metrics and alerts are keyed by.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A strategy instance's id: a slug of 1-32 lowercase ASCII letters, digits
/// and `-`, starting and ending with a letter or digit. `default` is
/// reserved for the legacy single-strategy config ([`InstanceId::default_instance`]):
/// its files stay at the state dir root and its names are unchanged.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct InstanceId(String);

/// The reserved id of the legacy single instance.
pub const DEFAULT_INSTANCE: &str = "default";

const MAX_LEN: usize = 32;

impl InstanceId {
    /// The legacy single instance (`default`).
    #[must_use]
    pub fn default_instance() -> Self {
        Self(DEFAULT_INSTANCE.to_string())
    }

    /// A configured instance id; `default` is refused (it is the legacy one).
    ///
    /// # Errors
    /// Not a slug, or `default`.
    pub fn parse(s: &str) -> Result<Self, String> {
        if s == DEFAULT_INSTANCE {
            return Err(format!(
                "instance id {s:?} is reserved for the single-strategy config; pick another name"
            ));
        }
        Self::checked(s)
    }

    fn checked(s: &str) -> Result<Self, String> {
        let slug = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
        let ok = !s.is_empty()
            && s.len() <= MAX_LEN
            && s.chars().all(slug)
            && !s.starts_with('-')
            && !s.ends_with('-');
        if ok {
            Ok(Self(s.to_string()))
        } else {
            Err(format!(
                "instance id {s:?} must be 1-{MAX_LEN} lowercase letters, digits or '-', starting \
                 and ending with a letter or digit"
            ))
        }
    }

    /// The legacy single instance: root paths, unchanged names.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.0 == DEFAULT_INSTANCE
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stored ids (a config version's document) may be `default`.
impl TryFrom<String> for InstanceId {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        Self::checked(&s)
    }
}

impl From<InstanceId> for String {
    fn from(id: InstanceId) -> Self {
        id.0
    }
}

impl fmt::Display for InstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a control call or read covers: one instance, or all of them (reads,
/// and the account-wide kill switch).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InstanceScope {
    One(InstanceId),
    All,
}

#[cfg(test)]
#[path = "instance_tests.rs"]
mod tests;
