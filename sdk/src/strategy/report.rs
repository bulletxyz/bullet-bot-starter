//! A strategy's own state as the host publishes it: the report. A strategy shows its fields; a
//! typed report (a struct that is `Serialize`) also hands the host its own JSON and schema, so
//! `/status`, MCP and the UI read exactly what they read before. A reader learns a strategy's
//! specifics only through the report's methods, never by a downcast or a JSON key.

use std::fmt;

use rust_decimal::Decimal;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};

/// The most a host takes from a plugin's report, in fields and characters: past them it faults.
pub const MAX_REPORT_FIELDS: usize = 128;
pub const MAX_FIELD_NAME: usize = 64;
pub const MAX_FIELD_TEXT: usize = 200;
pub const MAX_FIELD_UNIT: usize = 16;
pub const MAX_HEADLINE: usize = 200;

/// A strategy's own state, as the fields it shows: the host builds the report (and its schema)
/// from them, so a strategy never sends JSON or a schema of its own, unless it is a typed report
/// ([`Self::typed`]). What a generic reader needs is a method here, with a default when only some
/// strategies have it.
pub trait StrategyReport: fmt::Debug + Send + Sync + 'static {
    /// One line on what the strategy is doing now, in its own words.
    fn headline(&self) -> String;

    /// What it shows of itself, in order: at most [`MAX_REPORT_FIELDS`] fields, a name of 1 to
    /// [`MAX_FIELD_NAME`] characters, a text of at most [`MAX_FIELD_TEXT`] and a unit of at most
    /// [`MAX_FIELD_UNIT`].
    fn fields(&self) -> Vec<ReportField>;

    /// The report as its own `Serialize` struct, for a native strategy whose JSON a reader already
    /// knows. `None` (the default): the JSON is built from [`Self::fields`] ([`fields_json`]).
    fn typed(&self) -> Option<&dyn erased_serde::Serialize> {
        None
    }

    /// The report's JSON: its own struct's when it is typed, else its fields nested by dotted name.
    fn json(&self) -> Value {
        match self.typed() {
            Some(typed) => serde_json::to_value(typed).unwrap_or(Value::Null),
            None => fields_json(&self.fields()),
        }
    }

    /// The JSON schema of [`Self::json`] (`schemars::schema_for!(Self)` for a typed report): a
    /// reader may label the report's fields from it. The default accepts any object.
    fn schema(&self) -> schemars::Schema {
        schemars::json_schema!({ "type": "object" })
    }

    /// What the paper report needs from a strategy that quotes: `None` for one that doesn't.
    fn paper_inputs(&self) -> Option<PaperInputs> {
        None
    }

    /// Sentences the strategy says about itself in the briefing, each ending in a space, in order.
    fn briefing_lines(&self) -> Vec<String> {
        Vec::new()
    }

    /// The price the strategy values its own orders at without a fresh book (a market maker's fair
    /// value). `None`: it has none of its own.
    fn reference_price(&self) -> Option<Decimal> {
        None
    }

    /// Whether the report's own text (headline, briefing lines, JSON) was written by a plugin, not
    /// by this bot's code: a reader shows it as untrusted, never as the bot's words.
    fn written_by_plugin(&self) -> bool {
        false
    }
}

/// A report serializes as its JSON: its own struct's when it is typed (byte for byte what the
/// struct serializes to), else its fields.
impl Serialize for dyn StrategyReport + '_ {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.typed() {
            Some(typed) => erased_serde::serialize(typed, serializer),
            None => fields_json(&self.fields()).serialize(serializer),
        }
    }
}

/// `fields` as a JSON object, nested by dotted name (`a.b` is `b` in `a`); a number is its decimal
/// text, as a `Decimal` serializes.
#[must_use]
pub fn fields_json(fields: &[ReportField]) -> Value {
    let mut root = Map::new();
    for field in fields {
        let value = match &field.value {
            FieldValue::Number(n) => Value::String(n.to_string()),
            FieldValue::Text(t) => Value::String(t.clone()),
            FieldValue::Flag(b) => Value::Bool(*b),
        };
        insert(&mut root, &field.name.split('.').collect::<Vec<_>>(), value);
    }
    Value::Object(root)
}

/// `value` at `path` under `at`, making each object on the way (a leaf on the way is replaced).
fn insert(at: &mut Map<String, Value>, path: &[&str], value: Value) {
    match path {
        [] => {}
        [leaf] => {
            at.insert((*leaf).to_owned(), value);
        }
        [head, rest @ ..] => {
            let entry = at.entry((*head).to_owned()).or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            if let Value::Object(next) = entry {
                insert(next, rest, value);
            }
        }
    }
}

/// `report` (any `Serialize` struct) as the fields a plugin-style report shows: each top-level
/// number, text and flag in order; anything nested is shown as its JSON text.
#[must_use]
pub fn fields_of<T: Serialize>(report: &T) -> Vec<ReportField> {
    let Ok(Value::Object(map)) = serde_json::to_value(report) else { return Vec::new() };
    map.into_iter()
        .map(|(name, value)| match value {
            Value::Bool(b) => ReportField::flag(name, b),
            Value::String(s) => match s.parse::<Decimal>() {
                Ok(n) => ReportField::number(name, n),
                Err(_) => ReportField::text(name, s),
            },
            Value::Number(n) => match n.to_string().parse::<Decimal>() {
                Ok(n) => ReportField::number(name, n),
                Err(_) => ReportField::text(name, n.to_string()),
            },
            other => ReportField::text(name, other.to_string()),
        })
        .collect()
}

/// The paper report's inputs only a quoting strategy has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaperInputs {
    /// The size of one order.
    pub order_size: Decimal,
    /// Seconds this run spent paper trading with a quote up and no stop in force.
    pub quoting_secs: u64,
    /// The maker fee the config names, in basis points (0 when none is configured).
    pub maker_fee_bps: Decimal,
}

/// One field of a report: a name, its value and its unit, if it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportField {
    pub name: String,
    pub value: FieldValue,
    pub unit: Option<String>,
}

/// A report field's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    Number(Decimal),
    Text(String),
    Flag(bool),
}

impl ReportField {
    /// A number named `name`.
    #[must_use]
    pub fn number(name: impl Into<String>, value: Decimal) -> Self {
        Self { name: name.into(), value: FieldValue::Number(value), unit: None }
    }

    /// A text named `name`.
    #[must_use]
    pub fn text(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self { name: name.into(), value: FieldValue::Text(value.into()), unit: None }
    }

    /// A flag named `name`.
    #[must_use]
    pub fn flag(name: impl Into<String>, value: bool) -> Self {
        Self { name: name.into(), value: FieldValue::Flag(value), unit: None }
    }

    /// This field in `unit`.
    #[must_use]
    pub fn unit(self, unit: impl Into<String>) -> Self {
        Self { unit: Some(unit.into()), ..self }
    }
}

/// In an `impl StrategyReport for T` where `T` is `Serialize`: the report is its own struct, so a
/// reader gets the JSON the struct serializes to, and its fields are that JSON's top level.
///
/// ```ignore
/// impl StrategyReport for MyReport {
///     bb_strategy_sdk::typed_report!();
///     fn headline(&self) -> String { /* ... */ }
/// }
/// ```
#[macro_export]
macro_rules! typed_report {
    () => {
        fn fields(&self) -> ::std::vec::Vec<$crate::ReportField> {
            $crate::fields_of(self)
        }

        fn typed(&self) -> ::std::option::Option<&dyn $crate::ErasedSerialize> {
            ::std::option::Option::Some(self)
        }
    };
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
