use bb_strategy_sdk::{Describe, FromSettings};

use crate::RsiMomentum;

#[test]
fn rsi_momentum_is_named_for_its_folder_and_lists_every_setting() {
    assert_eq!(RsiMomentum::NAME, "rsi-momentum");
    assert_eq!(RsiMomentum::VERSION, env!("CARGO_PKG_VERSION"));
    let d = RsiMomentum::describe();
    let paths: Vec<_> = d.settings.iter().map(|s| s.path.clone()).collect();
    let want: Vec<_> = crate::settings().into_iter().map(|s| s.path).collect();
    assert_eq!(paths, want);
    assert!(d.legs.is_empty());
}

#[test]
fn rsi_momentum_builds_from_its_settings_and_refuses_what_it_cannot_run() {
    let ok = r#"{"symbol": "BTC-USD", "max_position": "0.01", "interval": "15m"}"#;
    assert!(RsiMomentum::from_settings(ok).is_ok());
    let unknown =
        RsiMomentum::from_settings(r#"{"symbol": "BTC-USD", "max_position": "1", "x": 1}"#);
    assert!(unknown.is_err_and(|e| e.0.contains('x')));
    // Parsed, but refused by its own check.
    let crossed = r#"{"symbol": "BTC-USD", "max_position": "1", "short_below": "70"}"#;
    assert!(RsiMomentum::from_settings(crossed).is_err());
}

#[test]
fn the_rsi_momentum_profile_is_the_bots_descriptor() {
    let p = RsiMomentum::profile().unwrap();
    assert_eq!(p.about.title, "RSI momentum");
    assert!(p.supports_paper && p.has_loss_stop);
    assert_eq!(p.settings, crate::settings());
    assert_eq!((p.exchanges[0].role, p.exchanges[0].setting), ("venue", None));
    let featured: Vec<_> = p.about.featured.iter().map(|f| f.path).collect();
    assert_eq!(featured, ["rsi_period", "long_above", "exit_band"]);
}

#[test]
fn the_momentum_preset_caps_the_position_at_the_amount_and_stops_at_five_percent() {
    use bb_strategy_sdk::profile::PresetMarket;
    use rust_decimal::Decimal;
    let p = RsiMomentum::profile().unwrap().presets;
    let [btc] = p.as_slice() else { panic!("one preset: {p:?}") };
    assert_eq!((btc.id, btc.symbol, btc.needs_market), ("btc-rsi-momentum", Some("BTC-USD"), true));
    let market = PresetMarket { price: Decimal::from(80_000), step: Decimal::new(1, 4) };
    let s = (btc.settings)(Decimal::from(100), Some(market)).unwrap();
    assert_eq!(s["notional_usd"], "100");
    // $100 at $80k is 0.00125 BTC, floored to the 0.0001 step.
    assert_eq!(s["max_position"], "0.0012");
    assert_eq!(s["daily_loss_usd"], "5");
    assert_eq!(s["stale_secs"], "30");
    assert!((btc.settings)(Decimal::from(100), None).unwrap_err().contains("price"));
    let dear = PresetMarket { price: Decimal::from(400_000), step: Decimal::new(1, 3) };
    assert!((btc.settings)(Decimal::from(100), Some(dear)).unwrap_err().contains("too small"));
    assert!((btc.worst_case)(Decimal::from(100)).contains("$5 loss"));
}

/// What the bot wired by name: the mark feed, a two-tick resync and a flatten at wind-down.
#[test]
fn rsi_momentum_reads_the_mark_resyncs_in_two_ticks_and_flattens_on_the_way_out() {
    use bb_strategy_sdk::{Feed, Needs};
    let needs = RsiMomentum::needs();
    assert_eq!(
        needs,
        Needs { feeds: vec![Feed::Mark], resync: Some(2), flatten_on_wind_down: true }
    );
}

/// What the bot took as rsi-momentum's own limits by name: its position cap and its loss stop.
#[test]
fn rsi_momentum_names_the_host_limits_its_position_cap_and_loss_stop_set() {
    use bb_strategy_sdk::HostLimit;
    let caps: Vec<_> =
        crate::settings().into_iter().filter_map(|s| s.caps.map(|c| (s.path, c))).collect();
    assert_eq!(
        caps,
        [
            ("max_position".to_string(), HostLimit::MaxPosition),
            ("daily_loss_usd".to_string(), HostLimit::DailyLossUsd)
        ]
    );
}
