use super::*;

#[test]
fn slugs_are_accepted() {
    for id in ["mm-btc", "a", "farb2", "0x", &"a".repeat(32)] {
        assert_eq!(InstanceId::parse(id).unwrap().as_str(), id);
    }
}

#[test]
fn non_slugs_are_refused() {
    for id in ["", "MM", "mm_btc", "mm btc", "-mm", "mm-", "../x", "a/b", &"a".repeat(33)] {
        assert!(InstanceId::parse(id).is_err(), "{id:?} accepted");
    }
}

#[test]
fn default_is_reserved_for_the_legacy_instance() {
    let e = InstanceId::parse("default").unwrap_err();
    assert!(e.contains("reserved"), "{e}");
    let d = InstanceId::default_instance();
    assert!(d.is_default());
    assert!(!InstanceId::parse("mm-btc").unwrap().is_default());
    let stored: InstanceId = serde_json::from_str("\"default\"").unwrap();
    assert_eq!(stored, d, "a stored document may name the legacy instance");
    assert!(serde_json::from_str::<InstanceId>("\"../etc\"").is_err());
}
