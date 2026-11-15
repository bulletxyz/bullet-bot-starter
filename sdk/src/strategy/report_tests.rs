use serde::Serialize;

use super::*;

#[derive(Debug, Serialize)]
struct Typed {
    mid: Decimal,
    levels: Vec<u32>,
    halted: bool,
}

impl StrategyReport for Typed {
    typed_report!();

    fn headline(&self) -> String {
        "typed".into()
    }
}

#[derive(Debug)]
struct Plain;

impl StrategyReport for Plain {
    fn headline(&self) -> String {
        "plain".into()
    }

    fn fields(&self) -> Vec<ReportField> {
        vec![
            ReportField::number("mid", Decimal::new(755, 1)),
            ReportField::text("book.state", "live"),
            ReportField::flag("book.crossed", false),
        ]
    }
}

/// A typed report serializes to exactly what its struct does, whether a reader asks for the JSON
/// value or serializes the trait object (`/status`, MCP and the UI serialize it, in field order).
#[test]
fn a_typed_report_is_its_own_structs_json() {
    let report = Typed { mid: Decimal::new(755, 1), levels: vec![1, 2], halted: false };
    let direct = serde_json::to_string(&report).unwrap();
    let erased: &dyn StrategyReport = &report;
    assert_eq!(serde_json::to_string(erased).unwrap(), direct);
    assert_eq!(report.json(), serde_json::to_value(&report).unwrap());
}

/// A report that only shows fields is built from them, nested by dotted name.
#[test]
fn a_fields_report_serializes_its_fields_nested_by_name() {
    let erased: &dyn StrategyReport = &Plain;
    let json = serde_json::to_value(erased).unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "mid": "75.5", "book": { "state": "live", "crossed": false } })
    );
    assert_eq!(Plain.json(), json);
}

/// A typed report's fields are its JSON's top level: numbers, texts and flags.
#[test]
fn a_typed_reports_fields_are_its_top_level() {
    let report = Typed { mid: Decimal::new(755, 1), levels: vec![1, 2], halted: true };
    let fields = report.fields();
    assert!(fields.contains(&ReportField::flag("halted", true)));
    assert!(
        fields
            .iter()
            .any(|f| f.name == "mid" && f.value == FieldValue::Number(Decimal::new(755, 1)))
    );
    assert!(
        fields
            .iter()
            .any(|f| f.name == "levels" && matches!(&f.value, FieldValue::Text(t) if t == "[1,2]"))
    );
}
