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

#[test]
fn a_built_ins_setting_crosses_as_the_setting_spec_of_its_kind() {
    use crate::Setting;
    use crate::param_spec::ParamSpec;
    let cap =
        ParamSpec::decimal("max_position", Decimal::ONE, Range::RUNTIME, Range::RUNTIME, "Cap.")
            .tier(Tier::Limit)
            .loosening(Loosening::Up);
    let spec = SettingSpec::from(&Setting::under("limits", cap).required());
    let by_hand = SettingSpec::decimal("limits.max_position", Decimal::ONE, Range::RUNTIME, "Cap.")
        .tier(Tier::Limit)
        .loosening(Loosening::Up)
        .required();
    assert_eq!(spec, by_hand);

    let kinds = [
        (ParamSpec::bool("on", true, "Help."), SettingKind::Flag),
        (
            ParamSpec::choice("side", &["a", "b"], 1, "Help."),
            SettingKind::Choice(vec!["a".into(), "b".into()]),
        ),
        (ParamSpec::text("venue", "bullet", "Help."), SettingKind::Text("bullet".into())),
        (
            ParamSpec::list("symbols", &["BTC-USD"], "Help."),
            SettingKind::Words(vec!["BTC-USD".into()]),
        ),
    ];
    for (param, kind) in kinds {
        assert_eq!(SettingSpec::from(&Setting::of(param)).kind, kind, "{}", param.name);
    }
}

/// Every strategy gets the host's position resync unless it says otherwise; no extra feed and no
/// flatten at wind-down.
#[test]
fn a_strategy_needs_only_the_resync_unless_it_says_more() {
    struct Bare;
    impl Describe for Bare {
        const NAME: &'static str = "bare";
        const VERSION: &'static str = "0.1.0";
        fn describe() -> Descriptor {
            Descriptor {
                summary: String::new(),
                settings: Vec::new(),
                settings_schema: schemars::Schema::default(),
                exchanges: vec![ExchangeNeed::new(BULLET)],
            }
        }
    }
    let needs = Bare::needs();
    assert_eq!(needs, Needs::default());
    assert!(needs.feeds.is_empty() && !needs.flatten_on_wind_down);
    assert_eq!(needs.resync, Some(DEFAULT_RESYNC_TICKS));
    assert_eq!(Bare::describe().check(), Ok(()));
}

/// The host runs only a strategy that trades Bullet, each venue named once.
#[test]
fn a_descriptor_must_trade_bullet_and_name_each_venue_once() {
    let cases: [(&[&str], Option<&str>); 5] = [
        (&[BULLET], None),
        (&["hyperliquid", BULLET], None),
        (&[], Some("must list it")),
        (&["hyperliquid"], Some("must list it")),
        (&[BULLET, "hyperliquid", BULLET], Some("bullet is listed twice")),
    ];
    for (venues, refused) in cases {
        let descriptor = Descriptor {
            summary: String::new(),
            settings: Vec::new(),
            settings_schema: schemars::Schema::default(),
            exchanges: venues.iter().map(|v| ExchangeNeed::new(v)).collect(),
        };
        match (descriptor.check(), refused) {
            (Ok(()), None) => {}
            (Err(why), Some(want)) => assert!(why.contains(want), "{venues:?}: {why}"),
            (got, want) => panic!("{venues:?}: {got:?}, wanted refusal {want:?}"),
        }
    }
}
