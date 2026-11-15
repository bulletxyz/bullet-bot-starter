use super::*;

// --- Direction: the starting position's IOC. ---

#[test]
fn long_opens_the_far_sides_levels_with_one_ioc_capped_at_the_touch() {
    let cfg = with(test_config(), serde_json::json!({ "direction": "long" }));
    let mut grid = GridStrategy::new(cfg);
    let b = mid_book();
    let intents = grid.start(&view(Some(&b), &[], NOON)).unwrap();
    let sent = placements(&intents);
    let ioc = sent[0];
    // Three sells above 75.5: buy 3 now, at the ask crossed by 20 bps (75.6 x 1.002).
    assert_eq!((ioc.side, ioc.quantity, ioc.order_type), (Side::Buy, d("3"), OrderType::Market));
    assert_eq!(ioc.price, d("75.7512"));
    assert!(!ioc.reduce_only);
    // The entry carries the hard stop (below the range: 74 - 2 widths of 4 x 0.5), so the venue
    // holds it from the first fill.
    assert_eq!(ioc.protect.and_then(|p| p.stop), Some(d("72")));
    // The sells go out once the IOC has filled, sized to what it bought: while flat, a sell would
    // open a short no stop guards.
    assert_eq!(sent.len(), 3, "the IOC and the two buys below");
    let short = with(test_config(), serde_json::json!({ "direction": "short" }));
    let intents = GridStrategy::new(short).start(&view(Some(&b), &[], NOON)).unwrap();
    let ioc = placements(&intents)[0];
    assert_eq!((ioc.side, ioc.quantity), (Side::Sell, d("2")), "the two buys below");
}

#[test]
fn a_refused_starting_ioc_stops_the_bot() {
    let cfg = with(test_config(), serde_json::json!({ "direction": "long" }));
    let mut grid = GridStrategy::new(cfg);
    let b = mid_book();
    let intents = grid.start(&view(Some(&b), &[], NOON)).unwrap();
    let ioc = placements(&intents)[0].clone();
    let refused = Event::Outcome {
        intent: Intent::Place(ioc.clone()),
        result: OrderResult {
            order_id: None,
            client_id: ioc.client_id,
            success: false,
            error: Some("risk gate: max notional".into()),
        },
    };
    let out = grid.on_event(&refused, &view(Some(&b), &[], NOON));
    assert!(
        matches!(out.as_slice(), [Intent::Shutdown(why)] if why.contains("max notional")),
        "{out:?}"
    );
}

// --- A directional grid never holds the side its hard stop does not guard. ---

#[test]
fn a_long_grid_sells_only_the_position_it_holds() {
    let long = with(test_config(), serde_json::json!({ "direction": "long" }));
    let mut grid = GridStrategy::new(long);
    let b = mid_book();
    let intents = grid.start(&view(Some(&b), &[], NOON)).unwrap();
    assert!(prices_of(&intents, Side::Sell).is_empty(), "flat: no sell: {intents:?}");
    let sent: Vec<NewOrder> = placements(&intents).into_iter().cloned().collect();
    for order in &sent {
        grid.on_event(&acked(order), &view(Some(&b), &[], NOON));
    }
    // The IOC bought 1.5 of its 3: one sell covers it, a second would open a short.
    let fill =
        Trade { quantity: d("1.5"), ..trade(sent[0].client_id.as_deref(), Side::Buy, "75.6") };
    grid.on_event(&Event::Fill(fill), &at(Some(&b), &[], NOON + 100, d("1.5")));
    let resting = resting(&sent[1..]);
    let out = grid.on_event(&Event::Tick, &at(Some(&b), &resting, NOON + 3_000, d("1.5")));
    assert_eq!(prices_of(&out, Side::Sell), ["76"], "{out:?}");
}

#[test]
fn a_short_grid_buys_only_the_position_it_holds() {
    let short = with(test_config(), serde_json::json!({ "direction": "short" }));
    let mut grid = GridStrategy::new(short);
    let b = mid_book();
    let intents = grid.start(&view(Some(&b), &[], NOON)).unwrap();
    assert!(prices_of(&intents, Side::Buy).is_empty(), "flat: no buy: {intents:?}");
    let sent: Vec<NewOrder> = placements(&intents).into_iter().cloned().collect();
    for order in &sent {
        grid.on_event(&acked(order), &view(Some(&b), &[], NOON));
    }
    // The IOC sold 1 of its 2: one buy covers it (the cap takes the levels lowest first).
    let resting = resting(&sent[1..]);
    let out = grid.on_event(&Event::Tick, &at(Some(&b), &resting, NOON + 3_000, d("-1")));
    assert_eq!(prices_of(&out, Side::Buy).len(), 1, "{out:?}");
}

#[test]
fn a_long_grid_whose_entry_never_landed_places_no_sell() {
    let long = with(test_config(), serde_json::json!({ "direction": "long" }));
    let mut grid = GridStrategy::new(long);
    let b = mid_book();
    let intents = grid.start(&view(Some(&b), &[], NOON)).unwrap();
    unknown_all(&mut grid, &intents, &b);
    let out = grid.on_event(&Event::Tick, &view(Some(&b), &[], NOON + 3_000));
    assert!(prices_of(&out, Side::Sell).is_empty(), "{out:?}");
}

#[test]
fn a_long_grid_whose_entry_was_accepted_but_unfilled_places_no_sell() {
    let long = with(test_config(), serde_json::json!({ "direction": "long" }));
    let (mut grid, sent) = running(long);
    assert_eq!(sent[0].order_type, OrderType::Market, "the entry");
    let resting = resting(&sent[1..]);
    let out = grid.on_event(&Event::Tick, &view(Some(&mid_book()), &resting, NOON + 3_000));
    assert!(prices_of(&out, Side::Sell).is_empty(), "{out:?}");
}
