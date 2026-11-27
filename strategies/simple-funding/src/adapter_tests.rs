use bb_strategy_sdk::{Answer, Event, ExchangeView, Intent, MarketView, StrategyAdapter, View};
use rust_decimal::Decimal;

use super::SimpleFunding;
use crate::{SimpleFundingActor, SimpleFundingConfig, VENUE};

const SYM: &str = "BTC-USD";

/// What the host shows: Bullet with `SYM` (a 0.001 size step), or no Bullet at all.
fn shown(bullet: bool) -> Vec<ExchangeView> {
    let market = MarketView {
        facts: bb_strategy_sdk::MarketFacts {
            step_size: Some(Decimal::new(1, 3)),
            ..Default::default()
        },
        ..MarketView::new(SYM)
    };
    if bullet { vec![ExchangeView::new(VENUE, vec![market])] } else { Vec::new() }
}

fn view(exchanges: &[ExchangeView]) -> View<'_> {
    View { now_ms: 1_000, exchanges, limits: None, stop: None }
}

/// Every request in `out` answered with no positions, until the flows ask nothing more.
fn answer_flat(strategy: &mut SimpleFunding, mut out: Vec<Intent>, exchanges: &[ExchangeView]) {
    for _ in 0..8 {
        let Some(id) = out.iter().find_map(|i| match i {
            Intent::Request { id, .. } => Some(*id),
            _ => None,
        }) else {
            return;
        };
        let answer = Event::Answer { id, reply: Ok(Answer::Positions(Vec::new())) };
        out = strategy.on_event(&answer, &view(exchanges));
    }
    panic!("the flows kept asking: {out:?}");
}

/// A view without the exchange is no data: its earlier facts are not acted on, and a tick asks nothing.
#[test]
fn a_view_without_the_exchange_starts_no_flow() {
    let config: SimpleFundingConfig = toml::from_str("").unwrap();
    let mut strategy = SimpleFunding::new(SimpleFundingActor::new(config, vec![SYM.into()]));
    let bullet = shown(true);
    let started = strategy.start(&view(&bullet)).unwrap();
    answer_flat(&mut strategy, started, &bullet);

    let none = shown(false);
    assert_eq!(strategy.on_event(&Event::Tick, &view(&none)), Vec::new(), "no exchange shown");
    assert!(strategy.shared.lock().symbols.is_empty(), "the earlier listing is cleared");

    let back = strategy.on_event(&Event::Tick, &view(&bullet));
    assert!(matches!(back.as_slice(), [Intent::Request { .. }]), "shown again, it reads: {back:?}");
}
