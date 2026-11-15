//! The entry reads the venue, not only the fills: the strategy alone, over hand-built views. The
//! venue's position comes with the start's read (`View::positions`); the marks after it carry none.

use bb_strategy_sdk::{Event, Intent, MarketFacts, StrategyAdapter, View};

use super::*;

/// What the long cross of [`LONG`] places when the start read the venue at `venue_position`
/// (`None`: the start could not read it).
fn entries(venue_position: Option<Decimal>) -> usize {
    let mut a = RsiMomentum::new(config(""));
    let market = MarketFacts { step_size: Some(d("0.001")), ..MarketFacts::default() };
    let books: Vec<OrderBook> = LONG.iter().chain(&["104"]).map(|p| book(d(p))).collect();
    let held: Vec<Position> = venue_position
        .filter(|p| !p.is_zero())
        .map(|p| {
            let side = if p > Decimal::ZERO { Side::Buy } else { Side::Sell };
            vec![Position { side: Some(side), size: p.abs(), ..long_pos("0", "100") }]
        })
        .unwrap_or_default();
    let view = |i: usize, positions: Option<&'static [Position]>| View {
        now_ms: T0 + u64::try_from(i).unwrap() * MIN + 1_000,
        symbol: SYM,
        book: Some(&books[i]),
        position: Decimal::ZERO,
        avg_entry: None,
        realized_pnl: Decimal::ZERO,
        open_orders: None,
        positions,
        reconnected: false,
        venue_halt: None,
        path_stop: None,
        read_started_ms: None,
        market: market.clone(),
        legs: &[],
    };
    let held: &'static [Position] = Box::leak(held.into_boxed_slice());
    a.start(&view(0, venue_position.map(|_| held))).unwrap();
    let mut placed = 0;
    for (i, price) in LONG.iter().chain(&["104"]).enumerate() {
        let out = a.on_event(&Event::Mark(d(price)), &view(i, None));
        placed += out.iter().filter(|i| matches!(i, Intent::Place(o) if !o.reduce_only)).count();
    }
    placed
}

#[test]
fn no_rsi_entry_while_the_venue_shows_a_position() {
    assert_eq!(entries(Some(d("0.5"))), 0, "a missed Trade: the venue holds it");
    assert_eq!(entries(Some(d("-0.5"))), 0, "either side");
    assert_eq!(entries(None), 0, "an unread venue is not a flat one");
    assert_eq!(entries(Some(Decimal::ZERO)), 1, "the venue reads flat: enter");
}
