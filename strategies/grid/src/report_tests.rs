use bb_strategy_sdk::Side;

use super::*;
use crate::levels::LevelState;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn grid_report(paused: bool) -> GridReport {
    GridReport {
        direction: Some("neutral".into()),
        range: "normal".into(),
        lower_price: Some(d("74")),
        upper_price: Some(d("78")),
        anchor: Some(d("75.5")),
        spacing: Some(d("1")),
        order_size: Some(d("1")),
        stop_below: Some(d("72")),
        stop_above: Some(d("80")),
        worst_case_usd: Some(d("231")),
        amount_usd: Some(d("1000")),
        active_levels: 1,
        total_levels: 2,
        paused,
        stopped: None,
        pnl: GridPnl { realized: d("1.5"), unrealized: d("-2"), total: d("-0.5") },
        levels: vec![
            GridLevel {
                index: 0,
                price: d("74"),
                side: Some(Side::Buy),
                state: LevelState::Active,
                client_id: Some("g-1".into()),
                order_id: Some("9".into()),
                filled_qty: d("0.5"),
            },
            GridLevel {
                index: 1,
                price: d("75"),
                side: None,
                state: LevelState::Dormant,
                client_id: None,
                order_id: None,
                filled_qty: Decimal::ZERO,
            },
        ],
        history: None,
    }
}

#[test]
fn the_headline_says_the_range_and_the_levels() {
    assert_eq!(grid_report(false).headline(), "1 of 2 levels resting between 74 and 78");
    assert_eq!(
        grid_report(true).headline(),
        "1 of 2 levels resting between 74 and 78, its opening orders paused"
    );
    let stopped = GridReport { stopped: Some("daily loss stop".into()), ..grid_report(false) };
    assert_eq!(stopped.headline(), "stopped: daily loss stop");
    assert_eq!(grid_report(false).paper_inputs(), None, "a grid doesn't quote one size");
}
