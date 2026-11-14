use super::*;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

#[expect(clippy::needless_pass_by_value, reason = "called with temporaries")]
fn cfg(v: serde_json::Value) -> GridConfig {
    let mut base = serde_json::json!({ "symbol": "SOL-USD" });
    for (k, x) in v.as_object().unwrap() {
        base[k] = x.clone();
    }
    serde_json::from_value(base).unwrap()
}

fn sol() -> Market {
    Market { mid: d("80"), tick: Some(d("0.001")), lot: Some(d("0.01")) }
}

fn candle(c: Decimal, spread: Decimal) -> Candle {
    Candle {
        open_ms: 0,
        open: c,
        high: c + spread,
        low: c - spread,
        close: c,
        volume: Decimal::ZERO,
    }
}

/// 31 daily closes alternating 80 and 83.2 (returns of +4% and -3.85%, σ about 4%), and 15 hourly
/// bars spanning 0.5 (ATR 0.5).
fn history() -> History {
    let daily =
        (0..31).map(|i| candle(if i % 2 == 0 { d("80") } else { d("83.2") }, d("1"))).collect();
    let hourly = (0..15).map(|_| candle(d("80"), d("0.25"))).collect();
    History { daily, hourly }
}

fn sides(p: &Plan) -> (usize, usize) {
    let (b, s) = layout((p.lower, p.upper, p.levels), p.anchor, Some(d("0.001")));
    (b.len(), s.len())
}

#[test]
fn derived_neutral_grid_centres_on_the_mid_with_its_worst_case_inside_the_dollars() {
    let p = resolve(&cfg(serde_json::json!({})), sol(), Some(&history()), HardLimits::default())
        .unwrap();
    // σ of the alternating returns is about 0.0398: 3σ = 11.9%, so 70.47 to 89.53.
    assert!(p.lower > d("70") && p.lower < d("71"), "{}", p.lower);
    assert!(p.upper > d("89") && p.upper < d("90"), "{}", p.upper);
    // Step: 0.77 x ATR 0.5 = 0.385 > 0.08 and 0.005; the range holds 50 such steps, capped at 50.
    assert_eq!(p.levels, derive::MAX_LEVELS);
    assert!(p.spacing() >= d("0.385"), "{}", p.spacing());
    let (buys, sells) = sides(&p);
    assert!(buys + sells >= p.levels - 1, "at most the level nearest the mid is empty");
    assert!(buys.abs_diff(sells) <= 1, "centred: {buys} buys, {sells} sells");
    assert!(p.entry.is_none(), "neutral starts flat");
    assert!(p.worst_notional <= d("100"), "{}", p.worst_notional);
    assert!(p.worst_notional > d("80"), "used, down to the lot: {}", p.worst_notional);
    assert_eq!(p.order_size, (p.order_size / d("0.01")).floor() * d("0.01"), "on the lot");
    assert_eq!(p.stop_below, Some(p.lower - (p.upper - p.lower) / d("2")));
    assert_eq!(p.stop_above, Some(p.upper + (p.upper - p.lower) / d("2")));
}

#[test]
fn long_buys_the_levels_above_the_mid_and_its_worst_case_is_every_level() {
    let c = cfg(serde_json::json!({ "direction": "long" }));
    let p = resolve(&c, sol(), Some(&history()), HardLimits::default()).unwrap();
    let (buys, sells) = sides(&p);
    assert_eq!(p.entry, Some((Side::Buy, p.order_size * Decimal::from(sells as u64))));
    assert_eq!(p.worst_coins, p.order_size * Decimal::from((buys + sells) as u64));
    assert!(p.worst_notional <= d("100"), "{}", p.worst_notional);
    assert!(p.stop_above.is_none() && p.stop_below.is_some(), "long stops below only");
    let short = resolve(
        &cfg(serde_json::json!({ "direction": "short" })),
        sol(),
        Some(&history()),
        HardLimits::default(),
    )
    .unwrap();
    let (buys, _) = sides(&short);
    assert_eq!(short.entry, Some((Side::Sell, short.order_size * Decimal::from(buys as u64))));
    assert!(short.stop_below.is_none() && short.stop_above.is_some(), "short stops above only");
}

#[test]
fn small_dollars_drop_levels_until_each_order_is_a_lot_and_too_small_is_refused() {
    let c = cfg(serde_json::json!({ "amount_usd": "10" }));
    let p = resolve(&c, sol(), Some(&history()), HardLimits::default()).unwrap();
    assert!(p.order_size >= d("0.01"));
    assert!(p.worst_notional <= d("10"), "{}", p.worst_notional);
    // $1 at 80 can't hold two orders of 0.01 SOL ($0.80 each) on one side plus the other.
    let tiny = cfg(serde_json::json!({ "amount_usd": "1" }));
    let e = resolve(&tiny, sol(), Some(&history()), HardLimits::default()).unwrap_err();
    assert!(e.contains("$1 is too small"), "{e}");
}

/// Rule 1: the worst case is held to the dollars and every hard limit, or the start is refused.
#[test]
fn a_worst_case_past_the_hard_limits_or_the_dollars_is_refused() {
    let explicit = || {
        cfg(serde_json::json!({
            "lower_price": "70", "upper_price": "90", "num_levels": 11, "order_size": "1",
            "amount_usd": "5000"
        }))
    };
    // Neutral, mid 80: buys 70..78 (5), sells 82..90 (5); short worst = 5 coins, $430.
    let p = resolve(&explicit(), sol(), None, HardLimits::default()).unwrap();
    assert_eq!((p.worst_coins, p.worst_notional), (d("5"), d("430")));
    let by_position = HardLimits { max_position: Some(d("4.99")), max_notional_usd: None };
    let e = resolve(&explicit(), sol(), None, by_position).unwrap_err();
    assert!(e.contains("position limit 4.99"), "{e}");
    let by_notional = HardLimits { max_position: None, max_notional_usd: Some(d("429")) };
    let e = resolve(&explicit(), sol(), None, by_notional).unwrap_err();
    assert!(e.contains("notional limit $429"), "{e}");
    let mut own = explicit();
    own.amount_usd = d("400");
    assert!(resolve(&own, sol(), None, HardLimits::default()).unwrap_err().contains("its $400"));
    let mut capped = explicit();
    capped.max_position = Some(d("4"));
    assert!(resolve(&capped, sol(), None, HardLimits::default()).is_err());
}

/// The deposit's notional limit is the dollars when it is lower than `amount_usd`: the real-money
/// choice sizes the grid without another setting.
#[test]
fn the_notional_limit_sizes_the_grid_when_it_is_below_the_amount() {
    let limits = HardLimits { max_position: None, max_notional_usd: Some(d("20")) };
    let p = resolve(&cfg(serde_json::json!({})), sol(), Some(&history()), limits).unwrap();
    assert_eq!(p.dollars, d("20"));
    assert!(p.worst_notional <= d("20"), "{}", p.worst_notional);
}

#[test]
fn no_history_or_a_mid_outside_the_range_refuses_the_start_in_plain_words() {
    let e = resolve(&cfg(serde_json::json!({})), sol(), None, HardLimits::default()).unwrap_err();
    assert!(e.contains("SOL-USD's price history"), "{e}");
    let out =
        cfg(serde_json::json!({ "lower_price": "90", "upper_price": "100", "num_levels": 5 }));
    let e = resolve(&out, sol(), None, HardLimits::default()).unwrap_err();
    assert!(e.contains("outside the grid's range"), "{e}");
    let no_tick = Market { tick: None, ..sol() };
    let e = resolve(&cfg(serde_json::json!({})), no_tick, Some(&history()), HardLimits::default())
        .unwrap_err();
    assert!(e.contains("tick"), "{e}");
}

#[test]
fn the_level_within_half_a_spacing_of_the_anchor_is_empty_and_a_halfway_anchor_empties_none() {
    let p = |anchor: &str| {
        let c = cfg(serde_json::json!({
            "lower_price": "74", "upper_price": "78", "num_levels": 5, "order_size": "1",
            "anchor_price": anchor, "amount_usd": "1000"
        }));
        let p = resolve(
            &c,
            Market { mid: d("75.5"), tick: None, lot: None },
            None,
            HardLimits::default(),
        )
        .unwrap();
        p.sides(None).into_iter().map(|(_, s)| s).collect::<Vec<_>>()
    };
    let (b, s) = (Some(Side::Buy), Some(Side::Sell));
    assert_eq!(p("76"), [b, b, None, s, s]);
    assert_eq!(p("75.9"), [b, b, None, s, s], "the nearest one");
    assert_eq!(p("75.5"), [b, b, s, s, s], "halfway between 75 and 76: none");
}

/// Prices on the tick can put two levels a hair under half a spacing from the mid: only the
/// nearer one is left empty.
#[test]
fn only_one_level_is_ever_empty() {
    let p = resolve(&cfg(serde_json::json!({})), sol(), Some(&history()), HardLimits::default())
        .unwrap();
    for tick in [Some(d("0.001")), Some(d("0.01")), Some(d("0.1")), None] {
        let empty = p.sides(tick).iter().filter(|(_, s)| s.is_none()).count();
        assert!(empty <= 1, "{tick:?}: {empty} empty");
    }
}
