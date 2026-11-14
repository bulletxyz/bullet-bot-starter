use super::*;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn close(c: &str) -> Candle {
    let p = d(c);
    Candle { open_ms: 0, open: p, high: p, low: p, close: p, volume: Decimal::ZERO }
}

fn bar(high: &str, low: &str, close: &str) -> Candle {
    Candle {
        open_ms: 0,
        open: d(close),
        high: d(high),
        low: d(low),
        close: d(close),
        volume: Decimal::ZERO,
    }
}

#[test]
fn sigma_is_the_sample_deviation_of_close_to_close_returns() {
    // Returns +10% and -10%: mean 0, sample variance (0.01 + 0.01) / 1 = 0.02.
    let s = daily_sigma(&[close("100"), close("110"), close("99")]).unwrap();
    assert!((s - 0.02_f64.sqrt()).abs() < 1e-12, "{s}");
    assert!(daily_sigma(&[close("100"), close("110")]).is_none(), "two closes: one return");
    assert!(daily_sigma(&[close("100"), close("0"), close("99")]).is_none());
}

#[test]
fn atr_averages_the_true_range_of_the_last_fourteen_hours() {
    // 15 bars: the first is only the previous close. Each later bar spans 2 but gaps 3 above the
    // close before it on the odd ones: true range max(2, |h - prev|, |l - prev|).
    let mut bars = vec![bar("101", "99", "100")];
    for i in 0..14 {
        bars.push(if i % 2 == 0 { bar("103", "101", "102") } else { bar("101", "99", "100") });
    }
    // Even: high 103 vs prev close 100 -> 3. Odd: low 99 vs prev 102 -> 3.
    assert_eq!(atr(&bars), Some(d("3")));
    assert!(atr(&bars[1..]).is_none(), "14 bars have only 13 ranges");
}

#[test]
fn the_range_is_three_sigma_inside_the_presets_band_on_the_tick() {
    let mid = d("80");
    let tick = d("0.001");
    // 3σ = 12% inside Normal's 3-47%: 70.4 to 89.6.
    assert_eq!(range(mid, 0.04, RangePreset::Normal, tick), (d("70.4"), d("89.6")));
    // A dead-calm week is held to Tight's 2%.
    assert_eq!(range(mid, 0.001, RangePreset::Tight, tick), (d("78.4"), d("81.6")));
    // A crash month to Normal's 47%, never down to zero.
    assert_eq!(range(mid, 0.5, RangePreset::Normal, tick), (d("42.4"), d("117.6")));
    // Wide allows the most.
    assert_eq!(range(mid, 0.5, RangePreset::Wide, tick), (d("32"), d("128")));
    assert_eq!(RangePreset::Tight.lookback_days(), 7);
    assert_eq!(RangePreset::Normal.lookback_days(), 30);
    assert_eq!(RangePreset::Wide.lookback_days(), 90);
}

#[test]
fn the_step_is_the_widest_of_atr_five_ticks_and_a_tenth_of_a_percent() {
    let tick = d("0.001");
    // 0.77 x 0.5 = 0.385 beats 0.005 and 0.08.
    assert_eq!(step(d("0.5"), tick, d("80")), d("0.385"));
    // A calm hour: 0.1% of the mid.
    assert_eq!(step(d("0.01"), tick, d("80")), d("0.08"));
    // A coarse tick: 5 ticks, and up to the tick.
    assert_eq!(step(d("1"), d("1"), d("100")), d("5"));
    assert_eq!(step(d("10"), d("1"), d("1000")), d("8"), "7.7 up to the tick");
}

#[test]
fn levels_follow_the_range_then_the_dollars_then_the_cap() {
    assert_eq!(levels(d("70"), d("90"), d("2"), usize::MAX), 11);
    assert_eq!(levels(d("70"), d("90"), d("2"), 6), 6, "the dollars hold six");
    assert_eq!(levels(d("70"), d("90"), d("0.1"), usize::MAX), MAX_LEVELS);
    // $100 at 80 with a 0.01 lot: 125 orders of one lot, plus the empty level.
    assert_eq!(levels_for_dollars(d("100"), d("0.01"), d("80")), 126);
    assert_eq!(levels_for_dollars(d("10"), d("0.0001"), d("100000")), 2);
}
