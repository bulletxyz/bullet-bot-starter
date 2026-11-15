use super::*;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

const SPREAD: ParamSpec = ParamSpec::decimal(
    "spread_bps",
    whole(10),
    Range::whole(2, 200),
    Range::whole(1, 1000),
    "distance from fair",
)
.loosening(Loosening::Down);

#[test]
fn guarded_hard_and_outside() {
    for mode in [BoundsMode::Guarded, BoundsMode::Extended] {
        assert_eq!(SPREAD.check(d("2"), mode), Ok(()));
        assert_eq!(SPREAD.check(d("200"), mode), Ok(()));
    }
    let guarded = Violation::OutOfBounds { min: d("2"), max: d("200") };
    assert_eq!(SPREAD.check(d("1.5"), BoundsMode::Guarded), Err(guarded));
    assert_eq!(SPREAD.check(d("1.5"), BoundsMode::Extended), Ok(()));
    assert_eq!(SPREAD.check(d("999"), BoundsMode::Extended), Ok(()));
    let hard = Violation::OutOfBounds { min: d("1"), max: d("1000") };
    for mode in [BoundsMode::Guarded, BoundsMode::Extended] {
        assert!(SPREAD.check(d("0.5"), mode).is_err());
    }
    assert_eq!(SPREAD.check(d("1001"), BoundsMode::Extended), Err(hard));
}

#[test]
fn bool_and_enum_values() {
    let on = ParamSpec::bool("widen", false, "");
    assert_eq!(on.default, Decimal::ZERO);
    assert_eq!(on.check(Decimal::ONE, BoundsMode::Guarded), Ok(()));
    assert_eq!(on.check(Decimal::ZERO, BoundsMode::Guarded), Ok(()));
    assert_eq!(on.check(d("2"), BoundsMode::Extended), Err(Violation::NotBool));
    assert_eq!(on.check(d("0.5"), BoundsMode::Extended), Err(Violation::NotBool));

    let pick = ParamSpec::choice("source", &["a", "b", "c"], 1, "");
    assert_eq!(pick.check(d("2"), BoundsMode::Guarded), Ok(()));
    assert_eq!(pick.check(d("1.5"), BoundsMode::Guarded), Err(Violation::NotInteger));
    assert!(matches!(pick.check(d("3"), BoundsMode::Extended), Err(Violation::OutOfBounds { .. })));
    assert!(pick.check_declaration().is_ok() && on.check_declaration().is_ok());
}

#[test]
fn int_must_be_whole() {
    let secs = ParamSpec::int("secs", 5, Range::whole(1, 60), Range::whole(1, 600), "");
    assert_eq!(secs.check(d("2.5"), BoundsMode::Extended), Err(Violation::NotInteger));
    assert_eq!(secs.check(d("2"), BoundsMode::Guarded), Ok(()));
}

#[test]
fn text_is_listed_not_bounded() {
    let venue = ParamSpec::text("hedge_exchange", "hyperliquid", "");
    assert_eq!(venue.kind, ParamKind::Text { default: "hyperliquid" });
    assert!(venue.check_declaration().is_ok());
    assert_eq!(venue.check(d("7"), BoundsMode::Guarded), Ok(()));
}

#[test]
fn runtime_sides_are_not_checked_here() {
    let size = ParamSpec::decimal("size", whole(1), Range::RUNTIME, Range::RUNTIME, "");
    assert_eq!(size.check(d("1e6"), BoundsMode::Guarded), Ok(()));
    assert!(size.check_declaration().is_ok());
}

#[test]
fn declarations_are_checked() {
    assert!(SPREAD.check_declaration().is_ok());
    let narrow_hard = ParamSpec { hard: Range::whole(5, 100), ..SPREAD };
    assert!(narrow_hard.check_declaration().is_err());
    let default_outside = ParamSpec { default: whole(500), ..SPREAD };
    assert!(default_outside.check_declaration().unwrap_err().contains("default"));
    let wide_bool = ParamSpec { hard: Range::whole(0, 2), ..ParamSpec::bool("b", true, "") };
    assert!(wide_bool.check_declaration().is_err());
}

#[test]
fn loosening_direction() {
    assert!(SPREAD.loosens(d("10"), d("5")));
    assert!(!SPREAD.loosens(d("10"), d("20")));
    let up = SPREAD.loosening(Loosening::Up);
    assert!(up.loosens(d("10"), d("20")));
    assert!(!SPREAD.loosening(Loosening::None).loosens(d("10"), d("0")));
}
