//! funding-arb as a plain `StrategyAdapter`: a market view in, `Intent::Targets(decide(view))` out,
//! and nothing else (no order, no request, no alert): the executor carries the targets out.

#![expect(clippy::unwrap_used, reason = "test helpers")]

use bb_strategy_funding_arb::{FundingArb, decide};
use bb_strategy_sdk::targets::{Direction, Leg, MarketView, Params, SymbolView};
use bb_strategy_sdk::testing::{Market, on_bullet};
use bb_strategy_sdk::{Event, Intent, MarketEvent, StrategyAdapter};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

fn d(v: i64) -> Decimal {
    Decimal::from_i64(v).unwrap()
}

fn market() -> MarketView {
    MarketView {
        symbols: vec![SymbolView {
            symbol: "BTC-USD".into(),
            signal_8h: Some(Decimal::new(5, 3)),
            round_trip_cost: Some(Decimal::new(1, 4)),
            mark: d(100),
            bullet_oi_usd: Some(d(1_000_000)),
            leg_bullet: Leg::default(),
            leg_hl: Leg::default(),
        }],
        equity_bullet: Some(d(10_000)),
        equity_hl: Some(d(10_000)),
        foreign_notional_bullet: Decimal::ZERO,
        foreign_notional_hl: Decimal::ZERO,
        params: Params {
            notional_per_symbol: d(500),
            max_positions: 3,
            max_oi_pct: Decimal::new(1, 2),
            max_leverage: d(3),
            expected_hold_hours: 48,
        },
    }
}

/// No exchange shown: funding-arb reads only the market view its event carries.
fn shown() -> Market {
    Market::of(0, Vec::new())
}

#[test]
fn a_market_view_is_answered_with_the_targets_decide_computes() {
    let m = market();
    let want = decide(&m);
    assert_eq!(want[0].direction, Direction::ShortBullet);
    assert!(want[0].notional_usd > Decimal::ZERO, "the scenario must enter");
    let mut s = FundingArb;
    let out = s.on_event(&Event::MarketView(m), &shown().view());
    assert_eq!(out.len(), 1);
    let Intent::Targets(got) = &out[0] else { panic!("expected Targets, got {:?}", out[0]) };
    assert_eq!(got, &want);
}

#[test]
fn nothing_else_makes_it_act() {
    let m = shown();
    let mut s = FundingArb;
    assert!(s.start(&m.view()).unwrap().is_empty());
    assert!(s.on_event(&Event::Tick, &m.view()).is_empty());
    assert!(s.on_event(&on_bullet("BTC-USD", MarketEvent::Book), &m.view()).is_empty());
    assert!(s.stop(&m.view()).is_empty());
}
