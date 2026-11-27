use super::*;

/// An intent only the bot's host takes is not dropped on the way out of a component: the glue
/// refuses it (a trap), so what the strategy asked for is never silently left undone.
#[test]
fn an_intent_the_world_cannot_carry_is_an_error_not_a_drop() {
    use crate::targets::{Direction, Target};
    for intent in [
        Intent::ClosePosition { id: 1, exchange: BULLET.into(), symbol: "BTC-USD".into() },
        Intent::StandDown("hold".into()),
        Intent::Targets(vec![Target {
            symbol: "BTC-USD".into(),
            direction: Direction::ShortBullet,
            notional_usd: Decimal::ONE,
        }]),
    ] {
        assert!(wire::intent_to_wire(intent.clone()).is_err(), "{intent:?}");
        let all = Intent::on(BULLET, ExchangeIntent::CancelAll(None));
        assert!(carry(vec![all, intent]).is_err());
    }
}

/// `Notify` is dropped rather than trapping the round, so its reduce-only close still goes out.
#[test]
fn a_stop_round_copied_from_a_built_in_keeps_its_close() {
    use crate::host::capability::Alert;
    let close = Intent::on(BULLET, ExchangeIntent::Place(order("99", "c-1")));
    let alert = Intent::Notify(Alert::Raise { key: "k".into(), text: "t".into() });
    let own = |venue, symbol: Option<&str>| {
        Intent::on(venue, ExchangeIntent::CancelOwn(symbol.map(Into::into)))
    };
    let got = carry(vec![
        own(BULLET, None),
        own("hyperliquid", Some("ETH")),
        close.clone(),
        alert.clone(),
    ]);
    let all = |venue: &str, symbol: Option<&str>| {
        w::Intent::On(w::OnExchange {
            exchange: venue.into(),
            intent: w::ExchangeIntent::CancelAll(symbol.map(Into::into)),
        })
    };
    let place = wire::intent_to_wire(close).unwrap();
    let want = [all(BULLET, None), all("hyperliquid", Some("ETH")), place];
    assert_eq!(format!("{:?}", got.unwrap()), format!("{want:?}"));
    // Only the glue maps them: the wire itself still has no case for either.
    assert!(wire::intent_to_wire(own(BULLET, None)).is_err());
    assert!(wire::intent_to_wire(alert).is_err());
}
