use bb_strategy_sdk::profile::PresetMarket;
use bb_strategy_sdk::testing::d;
use bb_strategy_sdk::{Describe, Feed, FromSettings, HostLimit, Needs};
use rust_decimal::Decimal;

use crate::MarketMaker;

/// Its name is the bot's name for its market maker, not its folder's: v51 instances, config
/// sections (`[strategies.simple-mm]`) and log prefixes carry it.
#[test]
fn the_market_maker_keeps_the_bots_name_and_lists_every_setting() {
    assert_eq!(MarketMaker::NAME, "simple-mm");
    assert_eq!(MarketMaker::VERSION, env!("CARGO_PKG_VERSION"));
    let d = MarketMaker::describe();
    let paths: Vec<_> = d.settings.iter().map(|s| s.path.clone()).collect();
    let want: Vec<_> = crate::settings::settings().into_iter().map(|s| s.path).collect();
    assert_eq!(paths, want);
    assert!(d.legs.is_empty());
}

#[test]
fn the_market_maker_builds_from_its_settings_and_refuses_what_it_cannot_run() {
    let ok = r#"{"symbol": "BTC-USD", "bid_spread_bps": "10", "ask_spread_bps": "10",
                 "order_size": "0.001", "max_position": "0.01"}"#;
    assert!(MarketMaker::from_settings(ok).is_ok());
    let unknown = MarketMaker::from_settings(r#"{"symbol": "BTC-USD", "spread": 1}"#);
    assert!(unknown.is_err_and(|e| e.0.contains("spread")));
    let small = ok.replace("\"0.01\"", "\"0.0001\"");
    assert!(MarketMaker::from_settings(&small).is_err_and(|e| e.0.contains("largest position")));
}

#[test]
fn the_market_maker_profile_is_the_bots_descriptor() {
    let p = MarketMaker::profile().unwrap();
    assert_eq!(p.about.title, "Market maker");
    assert!(p.supports_paper && p.has_loss_stop);
    assert_eq!(p.settings, crate::settings::settings());
    let roles: Vec<_> = p.exchanges.iter().map(|e| (e.role, e.tradable, e.required)).collect();
    assert_eq!(roles, [("venue", true, true), ("reference", false, false)]);
    let featured: Vec<_> = p.about.featured.iter().map(|f| f.path).collect();
    assert_eq!(featured, ["bid_spread_bps", "ask_spread_bps", "refresh_secs"]);
}

/// The fair value's prices from the reference feed; otherwise what the host wires for any
/// strategy.
#[test]
fn the_market_maker_reads_the_reference_feed() {
    assert_eq!(MarketMaker::needs(), Needs { feeds: vec![Feed::Reference], ..Needs::default() });
}

/// Its position cap and its loss stop set the host's limits of the same names.
#[test]
fn the_market_maker_names_the_host_limits_its_cap_and_loss_stop_set() {
    let caps: Vec<_> = crate::settings::settings()
        .into_iter()
        .filter_map(|s| s.caps.map(|c| (s.path, c)))
        .collect();
    assert_eq!(
        caps,
        [
            ("max_position".to_string(), HostLimit::MaxPosition),
            ("daily_loss_usd".to_string(), HostLimit::DailyLossUsd)
        ]
    );
}

/// The presets the bot offered by name: sized from the amount on the market's price and step.
#[test]
fn the_presets_size_the_cap_the_quotes_and_the_stop_from_the_amount() {
    let p = MarketMaker::profile().unwrap().presets;
    let [btc, eth] = p.as_slice() else { panic!("two presets: {p:?}") };
    assert_eq!((btc.id, btc.symbol, btc.needs_market), ("btc-conservative", Some("BTC-USD"), true));
    assert_eq!((eth.id, eth.symbol), ("eth-balanced", Some("ETH-USD")));
    let market = PresetMarket { price: Decimal::from(80_000), step: d("0.0001") };
    let s = (btc.settings)(Decimal::from(1_000), Some(market)).unwrap();
    let got: Vec<(&str, &str)> = s.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(
        got,
        [
            ("ask_spread_bps", "30"),
            ("bid_spread_bps", "30"),
            ("daily_loss_usd", "50"),
            ("max_notional_usd", "1000"),
            ("max_position", "0.0125"),
            ("order_size", "0.0031"),
            ("skew_bps", "25"),
            ("stale_secs", "30"),
        ]
    );
    assert!((btc.settings)(Decimal::from(100), None).unwrap_err().contains("price"));
    let coarse = PresetMarket { step: d("0.001"), ..market };
    let small = (btc.settings)(Decimal::from(100), Some(coarse)).unwrap_err();
    assert!(small.contains("too small"), "{small}");
    assert!((eth.worst_case)(Decimal::from(200)).contains("$20 loss"));
}
