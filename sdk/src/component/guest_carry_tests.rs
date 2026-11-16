use super::*;

/// An intent only the bot's host takes is not dropped on the way out of a component: the glue
/// refuses it (a trap), so what the strategy asked for is never silently left undone.
#[test]
fn an_intent_the_world_cannot_carry_is_an_error_not_a_drop() {
    use crate::targets::{Direction, Target};
    for intent in [
        Intent::ClosePosition { id: 1 },
        Intent::Targets(vec![Target {
            symbol: "BTC-USD".into(),
            direction: Direction::ShortBullet,
            notional_usd: Decimal::ONE,
        }]),
    ] {
        assert!(wire::intent_to_wire(intent.clone()).is_err(), "{intent:?}");
        let on_a_leg = Intent::Leg { leg: "hedge".into(), intent: Box::new(intent.clone()) };
        assert!(wire::intent_to_wire(on_a_leg).is_err(), "{intent:?} on a leg");
        assert!(carry(vec![Intent::CancelAll, intent]).is_err());
    }
    // What the host would refuse anyway is still left out, as before.
    let nested = Intent::Leg {
        leg: "hedge".into(),
        intent: Box::new(Intent::Leg { leg: "x".into(), intent: Box::new(Intent::CancelAll) }),
    };
    assert!(wire::intent_to_wire(nested).unwrap().is_none());
}

/// `Notify` is dropped rather than trapping the round, so its reduce-only close still goes out.
#[test]
fn a_stop_round_copied_from_a_built_in_keeps_its_close() {
    use crate::host::capability::Alert;
    let close = order("99", "c-1");
    let alert = Intent::Notify(Alert::Raise { key: "k".into(), text: "t".into() });
    let own_on_leg = Intent::Leg { leg: "hedge".into(), intent: Box::new(Intent::CancelOwn) };
    let got =
        carry(vec![Intent::CancelOwn, own_on_leg, Intent::Place(close.clone()), alert.clone()]);
    let leg_all = w::Intent::Leg(w::OnLeg { leg: "hedge".into(), intent: w::LegIntent::CancelAll });
    let place = wire::intent_to_wire(Intent::Place(close)).unwrap().unwrap();
    let want = [w::Intent::CancelAll, leg_all, place];
    assert_eq!(format!("{:?}", got.unwrap()), format!("{want:?}"));
    // Only the glue maps them: the wire itself still has no case for either.
    assert!(wire::intent_to_wire(Intent::CancelOwn).is_err());
    assert!(wire::intent_to_wire(alert).is_err());
}
