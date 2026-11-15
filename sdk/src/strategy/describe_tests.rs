use super::*;

#[test]
fn a_flag_is_the_int_setting_a_struct_update_made_by_hand() {
    let by_hand = SettingSpec {
        kind: SettingKind::Flag,
        ..SettingSpec::int("on", 1, Range::new(Decimal::ZERO, Decimal::ONE), "Help.")
    };
    assert_eq!(SettingSpec::flag("on", true, "Help."), by_hand);
    assert_eq!(SettingSpec::flag("on", false, "Help.").default, Decimal::ZERO);
}

#[test]
fn a_choice_defaults_to_an_index_and_ranges_over_all_of_them() {
    let s = SettingSpec::choice("interval", &["1m", "5m", "1h"], 1, "Candle length.");
    assert_eq!(s.kind, SettingKind::Choice(vec!["1m".into(), "5m".into(), "1h".into()]));
    assert_eq!(
        (s.default, s.guarded.min, s.guarded.max),
        (Decimal::ONE, Some(Decimal::ZERO), Some(Decimal::TWO))
    );
    assert_eq!(s.hard, s.guarded);
    assert_eq!(s.tier, Tier::Tune);
}
