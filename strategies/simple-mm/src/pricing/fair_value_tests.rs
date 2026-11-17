#![expect(clippy::arithmetic_side_effects, reason = "test arithmetic")]

use super::*;
use crate::settings::MarketMakerSettings;

/// `ms` on the bot clock.
fn at(ms: u64) -> u64 {
    1_793_102_400_000 + ms
}

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn params(fair_value: FairValue) -> MmParams {
    let cfg: MarketMakerSettings = serde_json::from_value(serde_json::json!({
        "symbol": "BTC-USD", "bid_spread_bps": "2", "ask_spread_bps": "2",
        "order_size": "1", "max_position": "1",
    }))
    .unwrap();
    MmParams { fair_value, ..MmParams::from_settings(&cfg) }
}

/// Binance at 100 / 102 (mid 101), Bullet mid 101.5 every second from t = 0 to `secs`.
fn warmed(secs: u64, p: &MmParams) -> FairValueTracker {
    let mut t = FairValueTracker::default();
    for s in 0..=secs {
        t.on_reference(d("100"), d("102"), at(s * 1000));
        t.on_bullet_mid(d("101.5"), at(s * 1000), p);
    }
    t
}

#[test]
fn fair_is_binance_mid_plus_basis_once_warm() {
    let p = params(FairValue::Binance);
    let mut t = warmed(BASIS_WARMUP_SECS, &p);
    let fair = t.fair(d("101.5"), at(BASIS_WARMUP_SECS * 1000), &p);
    assert_eq!(fair, Fair { price: d("101.5"), source: FairValue::Binance, fallback: None });
    assert_eq!(t.basis_bps(d("101.5")), Some(d("49.26")));
    // Binance moves up 0.1; the slow basis barely moves, so fair follows Binance.
    let now = (BASIS_WARMUP_SECS + 1) * 1000;
    t.on_reference(d("100.1"), d("102.1"), at(now));
    t.on_bullet_mid(d("101.5"), at(now), &p);
    let fair = t.fair(d("101.5"), at(now), &p);
    assert_eq!(fair.source, FairValue::Binance);
    assert!(fair.price > d("101.59") && fair.price < d("101.60"), "{fair:?}");
}

#[test]
fn basis_ema_moves_half_way_in_one_half_life() {
    let p = params(FairValue::Binance);
    let mut t = FairValueTracker::default();
    // Basis 0 for long enough that the half-life EMA weight dominates.
    let late = 10 * p.basis_halflife_secs * 1000;
    for ms in [0, late] {
        t.on_reference(d("100"), d("100"), at(ms));
        t.on_bullet_mid(d("100"), at(ms), &p);
    }
    let then = late + p.basis_halflife_secs * 1000;
    t.on_reference(d("100"), d("100"), at(then));
    t.on_bullet_mid(d("100.1"), at(then), &p); // sample 0.1 (10 bps)
    assert_eq!(t.basis.unwrap().ema, d("0.05"));
}

/// The warm value is the average over the warm-up, not mostly the first sample: +10 bps once,
/// then 60 s at 0, reads well under 1 bps.
#[test]
fn warm_basis_is_the_warm_up_average() {
    let p = params(FairValue::Binance);
    let mut t = FairValueTracker::default();
    t.on_reference(d("100"), d("100"), at(0));
    t.on_bullet_mid(d("100.1"), at(0), &p);
    for s in 1..=BASIS_WARMUP_SECS {
        t.on_reference(d("100"), d("100"), at(s * 1000));
        t.on_bullet_mid(d("100"), at(s * 1000), &p);
    }
    let bps = t.basis_bps(d("100")).unwrap();
    assert!(bps.abs() < d("1"), "{bps}");
}

#[test]
fn falls_back_to_bullet_mid_without_binance() {
    let p = params(FairValue::Binance);
    let t = FairValueTracker::default();
    let fair = t.fair(d("101.5"), at(0), &p);
    assert_eq!(fair.price, d("101.5"));
    assert_eq!(fair.source, FairValue::Bullet);
    assert_eq!(fair.fallback, Some(Fallback::NoReference));
    assert_eq!(t.basis_bps(d("101.5")), None);
}

#[test]
fn falls_back_while_basis_warms_up() {
    let p = params(FairValue::Binance);
    let t = warmed(BASIS_WARMUP_SECS - 1, &p);
    let fair = t.fair(d("101.5"), at((BASIS_WARMUP_SECS - 1) * 1000), &p);
    assert_eq!((fair.price, fair.fallback), (d("101.5"), Some(Fallback::BasisWarming)));
}

#[test]
fn stale_binance_falls_back_and_restarts_warm_up() {
    let p = params(FairValue::Binance);
    let mut t = warmed(BASIS_WARMUP_SECS, &p);
    let stale = (BASIS_WARMUP_SECS + p.reference_stale_secs + 1) * 1000;
    let fair = t.fair(d("101.5"), at(stale), &p);
    assert_eq!(fair.price, d("101.5"));
    assert_eq!(fair.fallback, Some(Fallback::ReferenceStale { secs: 5 }));
    assert!(fair.fallback.unwrap().to_string().contains("Binance mid stale"));
    // A Bullet update while Binance is stale drops the basis: warm-up again.
    t.on_bullet_mid(d("101.5"), at(stale), &p);
    t.on_reference(d("100"), d("102"), at(stale + 1000));
    t.on_bullet_mid(d("101.5"), at(stale + 1000), &p);
    let fair = t.fair(d("101.5"), at(stale + 1000), &p);
    assert_eq!(fair.fallback, Some(Fallback::BasisWarming));
}

#[test]
fn bullet_setting_uses_bullet_mid_without_a_reason() {
    let p = params(FairValue::Bullet);
    let t = warmed(BASIS_WARMUP_SECS, &p);
    let fair = t.fair(d("101.7"), at(BASIS_WARMUP_SECS * 1000), &p);
    assert_eq!(fair, Fair { price: d("101.7"), source: FairValue::Bullet, fallback: None });
}

#[test]
fn divergent_binance_fair_falls_back_and_recovers() {
    let p = params(FairValue::Binance);
    let mut t = warmed(BASIS_WARMUP_SECS, &p); // Binance 101, basis +0.5
    let now = (BASIS_WARMUP_SECS + 1) * 1000;
    // Binance jumps to 102.01: fair 102.51 is 99.5 bps above Bullet's 101.5.
    t.on_reference(d("102"), d("102.02"), at(now));
    let fair = t.fair(d("101.5"), at(now), &p);
    assert_eq!(fair.price, d("101.5"));
    assert_eq!(fair.source, FairValue::Bullet);
    assert_eq!(fair.fallback, Some(Fallback::Diverged { bps: d("99.5") }));
    assert_eq!(
        fair.fallback.unwrap().to_string(),
        "Binance fair value diverges from Bullet by 99.5 bps"
    );
    // Exactly at the limit is still within it.
    let at_limit = MmParams { max_divergence_bps: d("99.5"), ..p.clone() };
    assert_eq!(t.fair(d("101.5"), at(now), &at_limit).fallback, None);
    // Back within the limit: Binance again.
    t.on_reference(d("101"), d("101.02"), at(now + 1000));
    let fair = t.fair(d("101.5"), at(now + 1000), &p);
    assert_eq!((fair.source, fair.fallback), (FairValue::Binance, None));
    assert_eq!(fair.price, d("101.51"));
}

/// Once warm, a sample beyond `max_divergence_bps` from the basis is left out; one within it is
/// averaged in.
#[test]
fn warm_basis_ignores_divergent_samples() {
    let p = params(FairValue::Binance); // max_divergence_bps 20
    let mut t = warmed(BASIS_WARMUP_SECS, &p); // basis +0.5
    let now = (BASIS_WARMUP_SECS + 1) * 1000;
    t.on_reference(d("102"), d("102.02"), at(now)); // sample -0.51: 100 bps off
    t.on_bullet_mid(d("101.5"), at(now), &p);
    assert_eq!(t.basis.unwrap().ema, d("0.5"));
    t.on_reference(d("100.9"), d("100.9"), at(now + 1000)); // sample 0.6: 10 bps off
    t.on_bullet_mid(d("101.5"), at(now + 1000), &p);
    assert!(t.basis.unwrap().ema > d("0.5"));
}

/// Binance mid 100 and Bullet mid `bullet` every second over `[from, to]` seconds; returns the
/// shifts reported (re-learn starts).
fn run(t: &mut FairValueTracker, p: &MmParams, bullet: &str, from: u64, to: u64) -> usize {
    let mut relearned = 0;
    for s in from..=to {
        t.on_reference(d("100"), d("100"), at(s * 1000));
        relearned += usize::from(t.on_bullet_mid(d(bullet), at(s * 1000), p).is_some());
    }
    relearned
}

/// A basis step of +30 bps that stays: Bullet mid at once, then after [`BASIS_RELEARN_SECS`] of
/// divergence the basis re-learns the new level.
#[test]
fn sustained_basis_shift_is_relearned() {
    let p = params(FairValue::Binance); // max_divergence_bps 20
    let mut t = FairValueTracker::default();
    assert_eq!(run(&mut t, &p, "100", 0, BASIS_WARMUP_SECS), 0);
    let start = BASIS_WARMUP_SECS + 1;
    assert_eq!(run(&mut t, &p, "100.3", start, start), 0);
    let fair = t.fair(d("100.3"), at(start * 1000), &p);
    assert!(matches!(fair.fallback, Some(Fallback::Diverged { .. })), "{fair:?}");

    // Just short of the re-learn time: still divergent, nothing reported.
    let last = start + BASIS_RELEARN_SECS - 1;
    assert_eq!(run(&mut t, &p, "100.3", start + 1, last), 0);
    let fair = t.fair(d("100.3"), at(last * 1000), &p);
    assert!(matches!(fair.fallback, Some(Fallback::Diverged { .. })), "{fair:?}");
    // At the re-learn time the shift is reported once and warm-up restarts.
    let relearn = last + 1;
    t.on_reference(d("100"), d("100"), at(relearn * 1000));
    let moved = t.on_bullet_mid(d("100.3"), at(relearn * 1000), &p);
    assert_eq!(moved, Some(d("29.9")));
    assert_eq!(t.fair(d("100.3"), at(relearn * 1000), &p).fallback, Some(Fallback::BasisWarming));
    // Warm again: priced off Binance with the new basis.
    let warm = relearn + BASIS_WARMUP_SECS;
    assert_eq!(run(&mut t, &p, "100.3", relearn + 1, warm), 0);
    let fair = t.fair(d("100.3"), at(warm * 1000), &p);
    assert_eq!((fair.source, fair.fallback), (FairValue::Binance, None));
    assert_eq!(t.basis_bps(d("100.3")), Some(d("29.91")));
}

/// A divergence shorter than [`BASIS_RELEARN_SECS`] recovers to the old basis without a re-warm.
#[test]
fn brief_divergence_keeps_the_basis() {
    let p = params(FairValue::Binance);
    let mut t = FairValueTracker::default();
    run(&mut t, &p, "100", 0, BASIS_WARMUP_SECS);
    let start = BASIS_WARMUP_SECS + 1;
    let end = start + BASIS_RELEARN_SECS - 60;
    assert_eq!(run(&mut t, &p, "100.3", start, end), 0);
    assert_eq!(run(&mut t, &p, "100", end + 1, end + 1), 0);
    let fair = t.fair(d("100"), at((end + 1) * 1000), &p);
    assert_eq!((fair.source, fair.fallback), (FairValue::Binance, None));
    assert_eq!(t.basis_bps(d("100")), Some(d("0")));
    // A later divergence starts its own clock.
    let again = end + 2;
    assert_eq!(run(&mut t, &p, "100.3", again, again + BASIS_RELEARN_SECS - 1), 0);
}

/// A second Bullet mid at the same instant as the first carries no weight.
#[test]
fn same_instant_sample_does_not_replace_the_basis() {
    let p = params(FairValue::Binance);
    let mut t = FairValueTracker::default();
    t.on_reference(d("100"), d("100"), at(0));
    t.on_bullet_mid(d("100.1"), at(0), &p);
    t.on_bullet_mid(d("100.3"), at(0), &p);
    assert_eq!(t.basis.unwrap().ema, d("0.1"));
}

#[test]
fn crossed_or_empty_binance_book_is_ignored() {
    let mut t = FairValueTracker::default();
    t.on_reference(d("102"), d("100"), at(0));
    t.on_reference(d("0"), d("100"), at(0));
    assert!(t.reference.is_none());
}
