#![expect(clippy::arithmetic_side_effects, reason = "test arithmetic")]

use bb_strategy_sdk::testing::{book, d};

use super::*;
use crate::settings::MarketMakerSettings;

const T0: u64 = 1_793_102_400_000;

fn settings() -> MarketMakerSettings {
    serde_json::from_value(serde_json::json!({
        "symbol": "BTC-USD", "bid_spread_bps": "10", "ask_spread_bps": "10",
        "order_size": "1", "max_position": "10", "fair_value": "bullet",
    }))
    .unwrap()
}

fn params(on: bool) -> MmParams {
    let mut p = MmParams::from_settings(&settings());
    p.extra.insert(VOL_WIDEN_ENABLED.into(), if on { Decimal::ONE } else { Decimal::ZERO });
    p
}

fn view<'a>(params: &'a MmParams, mid: &str, now_ms: u64) -> MarketView<'a> {
    MarketView {
        book: None,
        mid: Some(d(mid)),
        now_ms,
        net_position: Decimal::ZERO,
        max_position: d("10"),
        params,
        tick: None,
    }
}

/// Mids 100 then 100.2 (20 bps) 5 s apart; the quotes after them.
fn quotes_after_a_move(p: &MmParams) -> Quotes {
    let mut mm = Pricing::new("BTCUSDT".into());
    mm.on_book(&view(p, "100", T0));
    mm.on_book(&view(p, "100.2", T0 + 5_000));
    mm.quote(&view(p, "100.2", T0 + 5_000))
}

fn bps(mid: &str, bps: &str) -> Decimal {
    d(mid) * (Decimal::ONE + d(bps) / d("10000"))
}

/// Off (the default): a volatile mid quotes exactly the plain spread.
#[test]
fn off_quotes_as_without_it() {
    let q = quotes_after_a_move(&params(false));
    assert_eq!(q.bid.unwrap().price, bps("100.2", "-10"));
    assert_eq!(q.ask.unwrap().price, bps("100.2", "10"));
    // The switch's values unset (a section from before it): off too.
    let mut bare = params(false);
    bare.extra.clear();
    assert_eq!(quotes_after_a_move(&bare), q);
}

/// On: a 20 bps range over the 10 bps threshold adds 5 bps a side.
#[test]
fn on_widens_both_spreads_while_volatile() {
    let q = quotes_after_a_move(&params(true));
    assert_eq!(q.bid.unwrap().price, bps("100.2", "-15"));
    assert_eq!(q.ask.unwrap().price, bps("100.2", "15"));

    let mut calm = params(true);
    calm.extra.insert(VOL_THRESHOLD_BPS.into(), d("25"));
    let q = quotes_after_a_move(&calm);
    assert_eq!(q.bid.unwrap().price, bps("100.2", "-10"));
}

/// Mids older than the window no longer count.
#[test]
fn old_mids_leave_the_window() {
    let mut p = params(true);
    p.extra.insert(VOL_WINDOW_SECS.into(), d("5"));
    let mut mm = Pricing::new("BTCUSDT".into());
    mm.on_book(&view(&p, "100", T0));
    mm.on_book(&view(&p, "100.2", T0 + 6_000));
    let q = mm.quote(&view(&p, "100.2", T0 + 6_000));
    assert_eq!(q.bid.unwrap().price, bps("100.2", "-10"));
}

/// Long half the cap with 10 bps of skew: both quotes 5 bps lower around mid 100.
#[test]
fn skew_lowers_both_quotes_when_long() {
    let p = MmParams { skew_bps: d("10"), ..params(false) };
    let b = book("99", "101", "10");
    let v = MarketView { book: Some(&b), net_position: d("5"), ..view(&p, "100", T0) };
    let q = Pricing::new("BTCUSDT".into()).quote(&v);
    assert_eq!((q.bid.unwrap().price, q.ask.unwrap().price), (d("99.85"), d("100.05")));
    // Short: both higher; past the cap the shift stops at the full skew.
    let v = MarketView { book: Some(&b), net_position: d("-20"), ..view(&p, "100", T0) };
    let q = Pricing::new("BTCUSDT".into()).quote(&v);
    assert_eq!((q.bid.unwrap().price, q.ask.unwrap().price), (d("100"), d("100.2")));
}

/// Short and skewed on a one-tick-wide book: the buy-back bid is held one tick under the ask.
#[test]
fn skew_never_crosses_when_short() {
    let p = MmParams {
        bid_spread_bps: d("2"),
        ask_spread_bps: d("2"),
        skew_bps: d("10"),
        ..params(false)
    };
    let b = book("100000", "100006", "1");
    let v = MarketView {
        book: Some(&b),
        net_position: d("-0.9"),
        max_position: d("1"),
        tick: Some(d("0.1")),
        ..view(&p, "100003", T0)
    };
    let q = Pricing::new("BTCUSDT".into()).quote(&v);
    assert_eq!(q.bid.unwrap().price, d("100005.9"));
    assert!(q.ask.unwrap().price > d("100000"));
}

/// Without a known tick a quote that would cross joins the touch instead.
#[test]
fn without_a_tick_a_crossing_quote_joins_the_touch() {
    let p = params(false);
    let b = book("100", "100.01", "1");
    let v = MarketView { book: Some(&b), ..view(&p, "100.005", T0) };
    assert_eq!(v.keep_off_touch(Side::Buy, d("100.02")), d("100"));
    assert_eq!(v.keep_off_touch(Side::Sell, d("99.99")), d("100.01"));
    assert_eq!(v.keep_off_touch(Side::Buy, d("99.5")), d("99.5"));
}

fn reference(symbol: &str, bid: &str, ask: &str, received_ms: u64) -> ReferencePrice {
    let mid = (d(bid) + d(ask)) / Decimal::TWO;
    ReferencePrice { symbol: symbol.into(), mid, bid: d(bid), ask: d(ask), received_ms }
}

/// Binance at 99.4/99.6 and Bullet at mid 100 every second until the basis is warm.
fn warm(mm: &mut Pricing, p: &MmParams, symbol: &str) -> u64 {
    let mut now = T0;
    for s in 0..=fair_value::BASIS_WARMUP_SECS {
        now = T0 + s * 1000;
        mm.on_reference(&reference(symbol, "99.4", "99.6", now), &view(p, "100", now));
        mm.on_book(&view(p, "100", now));
    }
    now
}

/// Once warm, quotes centre on the Binance mid plus the basis; another symbol's prices are not
/// its reference.
#[test]
fn binance_fair_value_centres_quotes_once_basis_is_warm() {
    let p = MmParams { fair_value: FairValue::Binance, ..params(false) };
    let mut mm = Pricing::new("BTCUSDT".into());
    let now = warm(&mut mm, &p, "BTCUSDT");
    let next = now + 1000;
    mm.on_reference(&reference("ETHUSDT", "50", "50.2", next), &view(&p, "100", next));
    mm.on_reference(&reference("btcusdt", "99.5", "99.7", next), &view(&p, "100", next));
    let fair = mm.fair(&view(&p, "100", next)).unwrap();
    assert_eq!((fair.source, fair.fallback), (FairValue::Binance, None));
    assert!(fair.price > d("100.09") && fair.price < d("100.11"), "{fair:?}");

    let mut other = Pricing::new("BTCUSDT".into());
    warm(&mut other, &p, "ETHUSDT");
    assert_eq!(other.fair(&view(&p, "100", now)).unwrap().fallback, Some(Fallback::NoReference));
}

/// Staleness runs from when the update arrived, not from when the strategy saw it.
#[test]
fn a_backlogged_binance_update_counts_as_stale() {
    let p = MmParams { fair_value: FairValue::Binance, ..params(false) };
    let mut mm = Pricing::new("BTCUSDT".into());
    let now = warm(&mut mm, &p, "BTCUSDT");
    let late = now + 10_000;
    mm.on_reference(&reference("BTCUSDT", "99.4", "99.6", late - 6_000), &view(&p, "100", late));
    let fair = mm.fair(&view(&p, "100", late)).unwrap();
    assert_eq!(fair.fallback, Some(Fallback::ReferenceStale { secs: 5 }));
}

/// Binance prices are ignored while pricing off Bullet, and a switch to binance warms up from
/// scratch: a basis warmed before is never reused.
#[test]
fn switching_to_binance_warms_up_from_scratch() {
    let binance = MmParams { fair_value: FairValue::Binance, ..params(false) };
    let bullet = params(false);
    let mut mm = Pricing::new("BTCUSDT".into());
    let now = warm(&mut mm, &bullet, "BTCUSDT");
    mm.on_params(&bullet, &binance);
    assert_eq!(mm.fair(&view(&binance, "100", now)).unwrap().fallback, Some(Fallback::NoReference));

    let now = warm(&mut mm, &binance, "BTCUSDT");
    mm.on_params(&binance, &bullet);
    mm.on_params(&bullet, &binance);
    let fair = mm.fair(&view(&binance, "100", now)).unwrap();
    assert_eq!(fair.fallback, Some(Fallback::NoReference));
}

/// Before any Bullet price there is no fair value to claim.
#[test]
fn no_fair_value_before_a_bullet_price() {
    let p = params(false);
    let v = MarketView { mid: None, ..view(&p, "100", T0) };
    assert_eq!(Pricing::new("BTCUSDT".into()).fair(&v), None);
    assert_eq!(Pricing::new("BTCUSDT".into()).quote(&v), Quotes::default());
}
