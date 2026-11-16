use bb_strategy_sdk::testing::d;

use super::*;

fn defaults() -> MmParams {
    let settings: MarketMakerSettings = serde_json::from_value(serde_json::json!({
        "symbol": "BTC-USD", "bid_spread_bps": "10", "ask_spread_bps": "10",
        "order_size": "0.001", "max_position": "0.005", "max_divergence_bps": "50",
    }))
    .unwrap();
    MmParams::from_settings(&settings)
}

fn size() -> SizeBounds {
    SizeBounds { min: d("0.001"), max: d("0.0025") }
}

fn change(pairs: &[(&str, &str)], bounds: BoundsMode) -> Result<MmParams, String> {
    let mut c = SettingChanges { bounds, ..SettingChanges::default() };
    for (k, v) in pairs {
        c = c.set(*k, *v);
    }
    defaults().changed(&c, size())
}

#[test]
fn fair_value_parses_only_known_sources() {
    assert_eq!("Binance".parse::<FairValue>().unwrap(), FairValue::Binance);
    assert_eq!(" bullet ".parse::<FairValue>().unwrap(), FairValue::Bullet);
    assert!("hyperliquid".parse::<FairValue>().is_err());
}

/// Quoting tighter than the touch is the owner's choice: a spread under the guarded 2 bps is
/// refused for the agent alone (Guarded) and accepted once the owner signed (Extended), down to
/// 0.01 bps a side; zero or less never passes.
#[test]
fn the_owner_can_quote_tighter_than_one_bps() {
    let (g, x) = (BoundsMode::Guarded, BoundsMode::Extended);
    for name in ["bid_spread_bps", "ask_spread_bps"] {
        for v in ["0.3", "0.01", "1.5"] {
            assert!(change(&[(name, v)], g).is_err(), "{name}={v} guarded");
            assert!(change(&[(name, v)], x).is_ok(), "{name}={v} extended");
        }
        for v in ["0", "0.005", "-1"] {
            for mode in [g, x] {
                assert!(change(&[(name, v)], mode).is_err(), "{name}={v} {mode:?}");
            }
        }
    }
}

/// Inside guarded: accepted. Outside guarded but inside hard: refused in Guarded, accepted in
/// Extended. Outside hard: always refused. `order_size` keeps its runtime bounds in both.
#[test]
fn guarded_and_hard_bounds_by_mode() {
    let (g, x) = (BoundsMode::Guarded, BoundsMode::Extended);
    for mode in [g, x] {
        assert!(change(&[("bid_spread_bps", "150")], mode).is_ok());
        assert!(change(&[("vol_widen_bps", "50")], mode).is_ok());
        assert!(change(&[("order_size", "0.003")], mode).is_err());
        assert!(change(&[("vol_widen_bps", "501")], mode).is_err());
    }
    for (name, v) in [("bid_spread_bps", "1.5"), ("skew_bps", "300"), ("refresh_secs", "120")] {
        assert!(change(&[(name, v)], g).is_err(), "{name}={v} guarded");
        assert!(change(&[(name, v)], x).is_ok(), "{name}={v} extended");
    }
    assert_eq!(change(&[("vol_widen_bps", "200")], x).unwrap().extra["vol_widen_bps"], d("200"));
    for (name, v) in
        [("bid_spread_bps", "0.005"), ("ask_spread_bps", "1001"), ("refresh_secs", "601")]
    {
        for mode in [g, x] {
            assert!(change(&[(name, v)], mode).is_err(), "{name}={v} {mode:?}");
        }
    }
}

/// Only the changed params are checked: a section value outside the agent's range never blocks
/// an unrelated change.
#[test]
fn only_the_changed_params_are_checked() {
    let mut p = defaults();
    p.skew_bps = d("300");
    let c = SettingChanges::default().set("bid_spread_bps", "12");
    assert_eq!(p.changed(&c, size()).unwrap().bid_spread_bps, d("12"));
}

#[test]
fn a_switch_takes_only_zero_or_one() {
    for mode in [BoundsMode::Guarded, BoundsMode::Extended] {
        assert!(change(&[("vol_widen_enabled", "1")], mode).unwrap().flag("vol_widen_enabled"));
        assert!(!change(&[("vol_widen_enabled", "0")], mode).unwrap().flag("vol_widen_enabled"));
        for bad in ["2", "0.5", "-1"] {
            assert!(change(&[("vol_widen_enabled", bad)], mode).is_err(), "{bad}");
        }
    }
}

/// The built-ins' guarded ranges are the bot's, unchanged, every declaration is consistent and
/// every section default sits inside its hard range.
#[test]
fn builtin_specs_keep_their_bounds_and_are_consistent() {
    let guarded: Vec<_> = BUILTIN_SPECS
        .iter()
        .filter(|s| !matches!(s.kind, ParamKind::Enum { .. }))
        .map(|s| {
            (s.name, s.guarded.min.map(|v| v.to_string()), s.guarded.max.map(|v| v.to_string()))
        })
        .collect();
    let some = |s: &str| Some(s.to_string());
    assert_eq!(
        guarded,
        [
            ("bid_spread_bps", some("2"), some("200")),
            ("ask_spread_bps", some("2"), some("200")),
            ("order_size", None, None),
            ("refresh_secs", some("1"), some("60")),
            ("refresh_threshold_bps", some("0.5"), some("50")),
            ("skew_bps", some("0"), some("100")),
            ("basis_halflife_secs", some("10"), some("3600")),
            ("reference_stale_secs", some("1"), some("10")),
            ("max_divergence_bps", some("5"), some("500")),
        ]
    );
    let p = defaults();
    for s in BUILTIN_SPECS.iter().chain(PARAMS) {
        assert_eq!(s.check_declaration(), Ok(()), "{}", s.name);
        if let Some(v) = p.get(s.name).filter(|_| s.hard != Range::RUNTIME) {
            assert!(s.hard.contains(v), "{} default {v} inside hard", s.name);
        }
    }
}

/// The reply names each change, old to new.
#[test]
fn the_summary_says_each_change_old_to_new() {
    let c = SettingChanges::default().set("refresh_secs", "3").set("fair_value", "bullet");
    let next = defaults().changed(&c, size()).unwrap();
    assert_eq!(next.summary(&defaults(), &c), "fair_value binance → bullet, refresh_secs 5 → 3");
}
