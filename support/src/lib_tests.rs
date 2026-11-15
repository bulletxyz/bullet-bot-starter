use super::*;

#[test]
fn clip_keeps_short_text_and_cuts_long_text_to_max_characters() {
    assert_eq!(clip("short", 10), "short");
    let long = "é".repeat(300);
    let cut = clip(&long, MAX_REPORT_TEXT);
    assert_eq!(cut.chars().count(), MAX_REPORT_TEXT);
    assert!(cut.ends_with('…'));
}

#[test]
fn floor_to_step_rounds_down_and_is_zero_for_a_non_positive_step_or_quantity() {
    use rust_decimal::Decimal;
    let d = |s: &str| s.parse::<Decimal>().unwrap();
    assert_eq!(floor_to_step(d("3.37"), Decimal::ZERO), Decimal::ZERO, "step 0");
    assert_eq!(floor_to_step(d("3.37"), d("-0.1")), Decimal::ZERO, "negative step");
    assert_eq!(floor_to_step(Decimal::ZERO, d("0.1")), Decimal::ZERO, "qty 0");
    assert_eq!(floor_to_step(d("-3.37"), d("0.1")), Decimal::ZERO, "negative qty");
    assert_eq!(floor_to_step(d("0.09"), d("0.1")), Decimal::ZERO, "below one step");
    assert_eq!(floor_to_step(d("3.3"), d("0.1")), d("3.3"), "an exact multiple");
    assert_eq!(floor_to_step(d("3.37"), d("0.1")), d("3.3"), "rounded down");
}
