use std::collections::BTreeMap;

use bb_strategy_sdk::testing::d;
use serde_json::json;

use super::*;
use crate::settings::MarketMakerSettings;

fn report() -> MarketMakerReport {
    let settings: MarketMakerSettings = serde_json::from_value(json!({
        "symbol": "BTC-USD", "bid_spread_bps": "10", "ask_spread_bps": "10",
        "order_size": "0.001", "max_position": "0.005",
        "fees": {"maker_bps": "1.5", "min_spread_fee_multiplier": "1"},
    }))
    .unwrap();
    MarketMakerReport {
        params: MmParams::from_settings(&settings),
        extra_params: vec![ExtraParam {
            name: "vol_k",
            default: d("2"),
            min: d("0"),
            max: d("10"),
            help: "Width per unit of volatility.",
        }],
        maker_fee_bps: d("1.5"),
        paper_quoting_secs: 7200,
        fair_source: Some(FairValue::Binance),
        fair_fallback: None,
        fair_value: Some(d("64000.5")),
        basis_bps: Some(d("1.2")),
    }
}

/// The report's JSON is what MCP (`bot.report`) and the web UI show of a market maker: this
/// freezes its shape.
#[test]
fn the_market_maker_report_serializes_as_its_own_struct() {
    let mut r = report();
    r.params.extra = BTreeMap::from([("vol_k".to_string(), d("3"))]);
    let json = serde_json::to_value(&r as &dyn StrategyReport).unwrap();
    assert_eq!(
        json,
        json!({
            "params": {
                "bid_spread_bps": "10", "ask_spread_bps": "10", "order_size": "0.001",
                "refresh_secs": 5, "refresh_threshold_bps": "1", "skew_bps": "0",
                "fair_value": "binance", "basis_halflife_secs": 300, "reference_stale_secs": 5,
                "max_divergence_bps": "20", "extra": {"vol_k": "3"},
            },
            "extra_params": [{"name": "vol_k", "default": "2", "min": "0", "max": "10",
                              "help": "Width per unit of volatility."}],
            "maker_fee_bps": "1.5",
            "paper_quoting_secs": 7200,
            "fair_source": "binance",
            "fair_fallback": null,
            "fair_value": "64000.5",
            "basis_bps": "1.2",
        })
    );
    let fallback =
        MarketMakerReport { fair_fallback: Some(Fallback::ReferenceStale { secs: 5 }), ..report() };
    let json = serde_json::to_value(&fallback as &dyn StrategyReport).unwrap();
    assert_eq!(json["fair_fallback"], json!({"reference_stale": {"secs": 5}}));
    assert!(
        serde_json::to_value(fallback.schema()).unwrap()["properties"]["fair_fallback"].is_object()
    );
}

/// What the paper report reads of a market maker: its order size, time quoting and fee.
#[test]
fn its_paper_inputs_are_the_order_size_the_time_quoting_and_the_configured_fee() {
    let inputs = report().paper_inputs().unwrap();
    assert_eq!(
        (inputs.order_size, inputs.quoting_secs, inputs.maker_fee_bps),
        (d("0.001"), 7200, d("1.5"))
    );
}

/// The briefing's fair-value sentence, word for word as the bot says it.
#[test]
fn the_briefing_lines_are_the_fair_value_sentence() {
    let r = report();
    let line = "Quotes are centred on the Binance mid plus the average Bullet-Binance gap \
                (+1.2 bps). ";
    assert_eq!(r.briefing_lines(), [line]);
    assert_eq!(r.headline(), line.trim_end());
    assert_eq!(r.reference_price(), Some(d("64000.5")));
    let none = MarketMakerReport { basis_bps: None, ..report() };
    assert_eq!(
        none.fair_sentence(),
        "Quotes are centred on the Binance mid plus the average Bullet-Binance gap. "
    );
    let waiting = MarketMakerReport { fair_source: None, ..report() };
    assert_eq!(
        waiting.fair_sentence(),
        "Quotes will move to the Binance mid plus the average Bullet-Binance gap after about a \
         minute of averaging that gap, and stay on the Bullet mid until then. "
    );
    let mut bullet = MarketMakerReport { fair_source: None, ..report() };
    bullet.params.fair_value = FairValue::Bullet;
    assert_eq!(
        bullet.fair_sentence(),
        "Quotes will be centred on the Bullet mid once it has a price. "
    );
    let fallback = MarketMakerReport {
        fair_source: Some(FairValue::Bullet),
        fair_fallback: Some(Fallback::NoReference),
        ..report()
    };
    assert!(fallback.fair_sentence().starts_with("Quotes are centred on the Bullet mid for now: "));
    let plain = MarketMakerReport { fair_source: Some(FairValue::Bullet), ..report() };
    assert_eq!(plain.fair_sentence(), "Quotes are centred on the Bullet mid. ");
}
