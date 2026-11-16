use super::*;

const M: u64 = 60_000;
/// A UTC day boundary (so every interval's boundary).
const T0: u64 = 20_000 * 1_440 * M;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

#[test]
fn interval_lengths() {
    assert_eq!(CandleInterval::M1.ms(), M);
    assert_eq!(CandleInterval::M15.ms(), 15 * M);
    assert_eq!(CandleInterval::H4.ms(), 4 * 60 * M);
    assert_eq!(CandleInterval::D1.ms(), 24 * 60 * M);
}

#[test]
fn prices_inside_a_bucket_build_one_candle() {
    let mut b = CandleBuilder::new(CandleInterval::M1);
    for (t, p) in [(0, "100"), (10_000, "103"), (20_000, "98"), (59_999, "101")] {
        assert_eq!(b.on_price(T0 + t, d(p)), None);
    }
    let c = b.current().unwrap();
    assert_eq!(
        (c.open_ms, c.open, c.high, c.low, c.close),
        (T0, d("100"), d("103"), d("98"), d("101"))
    );
}

#[test]
fn the_first_price_in_the_next_bucket_closes_the_candle_on_the_boundary() {
    let mut b = CandleBuilder::new(CandleInterval::M1);
    b.on_price(T0 + 5_000, d("100"));
    b.on_price(T0 + 59_999, d("102"));
    let closed = b.on_price(T0 + M, d("104")).expect("the boundary closes it");
    assert_eq!(closed.missed, 0);
    assert_eq!(closed.candle.open_ms, T0);
    assert_eq!(closed.candle.close, d("102"), "the close is the last price before the boundary");
    assert_eq!(b.current().unwrap().open_ms, T0 + M);
    assert_eq!(b.current().unwrap().open, d("104"));
}

#[test]
fn buckets_align_to_the_interval_not_to_the_first_price() {
    let mut b = CandleBuilder::new(CandleInterval::M15);
    b.on_price(T0 + 7 * M, d("1"));
    assert_eq!(b.current().unwrap().open_ms % (15 * M), 0);
    // Still the same 15m bucket 7 minutes later; the next one closes it.
    assert_eq!(b.on_price(T0 + 14 * M, d("2")), None);
    assert!(b.on_price(T0 + 15 * M, d("3")).is_some());
}

#[test]
fn a_gap_reports_the_missed_buckets() {
    let mut b = CandleBuilder::new(CandleInterval::M1);
    b.on_price(T0, d("100"));
    let closed = b.on_price(T0 + 4 * M + 1, d("90")).unwrap();
    assert_eq!(closed.missed, 3, "minutes 1, 2 and 3 had no price");
    assert_eq!(closed.candle.close, d("100"));
}

#[test]
fn a_price_from_an_earlier_bucket_or_not_positive_is_ignored() {
    let mut b = CandleBuilder::new(CandleInterval::M1);
    b.on_price(T0 + M, d("100"));
    assert_eq!(b.on_price(T0 + 1_000, d("50")), None, "the clock stepped back");
    assert_eq!(b.on_price(T0 + M + 1, d("0")), None);
    assert_eq!(b.on_price(T0 + M + 2, d("-1")), None);
    let c = b.current().unwrap();
    assert_eq!((c.open_ms, c.low, c.close), (T0 + M, d("100"), d("100")));
}
