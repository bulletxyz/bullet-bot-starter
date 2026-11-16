use bb_strategy_sdk::testing::d;

use super::*;

fn caps(max_position: &str, notional: Option<&str>, step: Option<&str>) -> Caps {
    Caps { max_position: d(max_position), max_notional_usd: notional.map(d), step: step.map(d) }
}

/// At the cap the side that would grow the position is not quoted; the side that shrinks it is,
/// as long as it does not grow the other way past the cap.
#[test]
fn the_side_that_would_grow_past_the_cap_is_not_quoted() {
    let c = caps("1", None, None);
    let (size, price) = (d("0.1"), d("100"));
    assert_eq!(c.allowed(Side::Buy, size, price, d("0.9")), Some(size), "exactly to the cap");
    assert_eq!(c.allowed(Side::Buy, size, price, d("0.95")), None, "past it: dropped, not shrunk");
    assert_eq!(c.allowed(Side::Sell, size, price, d("0.95")), Some(size));
    assert_eq!(c.allowed(Side::Sell, size, price, d("-0.95")), None);
    assert_eq!(c.allowed(Side::Buy, size, price, d("-0.95")), Some(size));
}

/// One quote may be as big as the cap, never past it: a larger size is clamped to the cap in
/// whole steps, and one that rounds to nothing is dropped.
#[test]
fn a_quote_past_the_cap_is_clamped_to_it_in_whole_steps() {
    let c = caps("0.25", None, Some("0.1"));
    assert_eq!(c.allowed(Side::Buy, d("1"), d("100"), Decimal::ZERO), Some(d("0.2")));
    assert_eq!(
        caps("0.25", None, None).allowed(Side::Buy, d("1"), d("100"), d("0")),
        Some(d("0.25"))
    );
    assert_eq!(caps("0.05", None, Some("0.1")).allowed(Side::Sell, d("1"), d("100"), d("0")), None);
    assert_eq!(c.allowed(Side::Buy, Decimal::ZERO, d("100"), d("0")), None, "a zero size");
}

/// The notional cap drops the side whose fill would hold more dollars than it allows.
#[test]
fn the_notional_cap_drops_the_side_that_would_pass_it() {
    let c = caps("10", Some("150"), None);
    assert_eq!(c.allowed(Side::Buy, d("1"), d("100"), Decimal::ZERO), Some(d("1")));
    assert_eq!(c.allowed(Side::Buy, d("1"), d("100"), d("0.6")), None, "1.6 x 100 > 150");
    assert_eq!(c.allowed(Side::Sell, d("1"), d("100"), d("0.6")), Some(d("1")), "it shrinks");
    assert_eq!(c.allowed(Side::Buy, d("1"), Decimal::ZERO, Decimal::ZERO), None, "no price");
}

/// The host's live limits tighten the section's caps; they never loosen them.
#[test]
fn the_hosts_limits_tighten_the_caps() {
    let settings: MarketMakerSettings = serde_json::from_value(serde_json::json!({
        "symbol": "BTC-USD", "bid_spread_bps": "10", "ask_spread_bps": "10",
        "order_size": "0.1", "max_position": "1", "max_notional_usd": "500",
    }))
    .unwrap();
    let tight = LimitsInForce {
        max_notional_usd: Some(d("200")),
        ..LimitsInForce::new(d("0.5"), d("10"), Some(30))
    };
    let c = Caps::new(&settings, Some(&tight), Some(d("0.01")));
    assert_eq!((c.max_position, c.max_notional_usd), (d("0.5"), Some(d("200"))));
    let loose = LimitsInForce::new(d("5"), d("10"), Some(30));
    let c = Caps::new(&settings, Some(&loose), None);
    assert_eq!((c.max_position, c.max_notional_usd), (d("1"), Some(d("500"))));
    assert_eq!(Caps::new(&settings, None, None).max_position, d("1"));
}

/// A reprice is one message only if the replacement passes with the old quote counted as filled.
#[test]
fn near_the_cap_a_reprice_does_not_pass_with_the_old_quote_filled() {
    let c = caps("1", None, None);
    let (size, price) = (d("0.1"), d("100"));
    assert!(c.passes_with_filled(Side::Buy, size, price, d("0.5"), size));
    assert!(!c.passes_with_filled(Side::Buy, size, price, d("0.85"), size));
    assert!(!c.passes_with_filled(Side::Sell, size, price, d("-0.85"), size));
    assert!(c.passes_with_filled(Side::Sell, size, price, d("0.85"), size));
}
