use super::*;

#[test]
fn monotonic() {
    let mut iss = ClientIdIssuer::new();
    assert_eq!(iss.issue(), "1");
    assert_eq!(iss.issue(), "2");
    assert_eq!(iss.issue(), "3");
}

#[test]
fn default_starts_at_one() {
    // `Default` gives a zero'd `next`; the first `issue()` bumps to 1
    // so callers don't need to remember to call `new`.
    let mut iss = ClientIdIssuer::default();
    assert_eq!(iss.issue(), "1");
}

#[test]
fn starting_at_resumes_past_high_water_mark() {
    let mut iss = ClientIdIssuer::starting_at(1000);
    assert_eq!(iss.issue(), "1000");
    assert_eq!(iss.issue(), "1001");
}

#[test]
fn own_ids_are_marked_and_others_are_not() {
    let now_ms = 1_762_000_000_000;
    let mut iss = ClientIdIssuer::own(now_ms);
    let (a, b) = (iss.issue(), iss.issue());
    assert!(is_own(&a) && is_own(&b), "{a} {b}");
    assert!(b.parse::<u64>().unwrap() < 1 << 53, "{b} stays exact as a JSON number");
    assert!(ClientIdIssuer::own(now_ms + 1).issue() > b, "a later start issues above");
    // Before the mark: seconds x 10 000, a plugin's ms x 1 000, a close's ms x 10 000.
    for other in ["1", "17620000000000", "1762000000000000", "17620000000000000", "c1", ""] {
        assert!(!is_own(other), "{other}");
    }
    assert!(!is_own(&(OWN_MARK << SEQ_BITS | 1 << 53).to_string()), "nothing above the mark");
}

#[test]
fn starting_at_max_saturates_instead_of_overflowing() {
    let mut iss = ClientIdIssuer::starting_at(u64::MAX);
    assert_eq!(iss.issue(), u64::MAX.to_string());
    assert_eq!(iss.issue(), u64::MAX.to_string());
}
