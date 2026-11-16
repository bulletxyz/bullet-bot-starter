use super::*;

/// Tiny deterministic generator for the property test.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, one_in: u64) -> bool {
        self.below(one_in) == 0
    }
}

fn random_view(rng: &mut Rng) -> MarketView {
    let n = 1 + rng.below(14);
    let symbols = (0..n)
        .map(|i| {
            let signal = if rng.chance(6) {
                None
            } else {
                Some(Decimal::new(i64::try_from(rng.below(2001)).unwrap() - 1000, 6))
            };
            let held_usd = if rng.chance(3) {
                let sign = if rng.chance(2) { 1 } else { -1 };
                Decimal::from(i64::try_from(rng.below(160)).unwrap() * sign)
            } else {
                Decimal::ZERO
            };
            SymbolView {
                symbol: format!("M{i}"),
                signal_8h: signal,
                round_trip_cost: (!rng.chance(8))
                    .then(|| Decimal::new(i64::try_from(rng.below(40)).unwrap(), 4)),
                mark: if rng.chance(10) { Decimal::ZERO } else { d("50") },
                oi_usd: (!rng.chance(8)).then(|| Decimal::from(rng.below(5000))),
                depth_usd: (!rng.chance(8)).then(|| Decimal::from(rng.below(20_000))),
                fresh: !rng.chance(8),
                held_usd,
                entry_cost: (!held_usd.is_zero() && rng.chance(2))
                    .then(|| Decimal::new(i64::try_from(rng.below(40)).unwrap(), 4)),
            }
        })
        .collect();
    let params = Params {
        notional_per_symbol: Decimal::from(50 + rng.below(100)),
        top_k: 1 + usize::try_from(rng.below(4)).unwrap(),
        max_positions: 1 + usize::try_from(rng.below(8)).unwrap(),
        max_net_pct: Decimal::new(i64::try_from(rng.below(101)).unwrap(), 2),
        max_oi_pct: Decimal::new(1 + i64::try_from(rng.below(50)).unwrap(), 2),
        max_leverage: Decimal::from(1 + rng.below(4)),
        expected_hold_hours: 1 + rng.below(200),
        min_depth_usd: Decimal::from(rng.below(8000)),
    };
    MarketView {
        symbols,
        equity: (!rng.chance(10)).then(|| Decimal::from(rng.below(2000))),
        foreign_gross_usd: Decimal::from(rng.below(300)),
        params,
    }
}

#[test]
fn properties_hold_on_random_views() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for case in 0..4000 {
        let v = random_view(&mut rng);
        let p = &v.params;
        let out = decide(&v);
        assert_eq!(out.len(), v.symbols.len(), "case {case}: one target per symbol");
        let kept: Vec<Decimal> = v.symbols.iter().map(|s| keep(s, p)).collect();

        let mut new_long = 0;
        let mut new_short = 0;
        for ((s, t), kept) in v.symbols.iter().zip(&out).zip(&kept) {
            assert_eq!(s.symbol, t.symbol, "case {case}: view order");
            let n = t.notional_usd;
            if n == *kept {
                continue;
            }
            // Anything beyond a keep is an addition, and an addition is allowed only on a
            // known signal, cost, equity, fresh and deep market, in the signal's direction,
            // never above the leg target or the OI share, never reversing a hold.
            assert!(v.equity.is_some(), "case {case}: entry with unknown equity");
            assert!(s.fresh && s.signal_8h.is_some() && s.round_trip_cost.is_some(), "case {case}");
            assert!(s.depth_usd.is_some_and(|d| d >= p.min_depth_usd), "case {case}: thin book");
            assert!(s.mark > Decimal::ZERO, "case {case}");
            assert!(n.abs() <= p.notional_per_symbol, "case {case}: leg over the target");
            assert!(
                n.abs() <= p.max_oi_pct * s.oi_usd.unwrap(),
                "case {case}: leg over the OI share"
            );
            let leg = Leg::of_signal(s.signal_8h.unwrap()).unwrap();
            assert_eq!(Leg::of_notional(n), Some(leg), "case {case}: against the signal");
            assert!(
                carry(s.signal_8h.unwrap(), p.expected_hold_hours) > s.round_trip_cost.unwrap(),
                "case {case}: carry under the hurdle"
            );
            if s.held_usd.is_zero() {
                match leg {
                    Leg::Long => new_long += 1,
                    Leg::Short => new_short += 1,
                }
            } else {
                assert_eq!(Leg::of_notional(s.held_usd), Some(leg), "case {case}: reversed a hold");
                assert!(n.abs() > s.held_usd.abs(), "case {case}: a top-up only grows");
            }
        }
        // An unknown signal never changes a position.
        for (s, t) in v.symbols.iter().zip(&out) {
            if s.signal_8h.is_none() {
                assert_eq!(
                    t.notional_usd, s.held_usd,
                    "case {case}: unknown signal moved a position"
                );
            }
        }
        // Caps.
        let open = out.iter().filter(|t| !t.notional_usd.is_zero()).count();
        let kept_open = kept.iter().filter(|k| !k.is_zero()).count();
        assert!(open <= p.max_positions.max(kept_open), "case {case}: max_positions");
        let kept_long = kept.iter().filter(|k| **k > Decimal::ZERO).count();
        let kept_short = kept.iter().filter(|k| **k < Decimal::ZERO).count();
        assert!(new_long == 0 || kept_long + new_long <= p.top_k, "case {case}: top_k long");
        assert!(new_short == 0 || kept_short + new_short <= p.top_k, "case {case}: top_k short");
        let gross: Decimal = out.iter().map(|t| t.notional_usd.abs()).sum();
        let kept_gross: Decimal = kept.iter().map(Decimal::abs).sum();
        let budget = v.equity.map_or(Decimal::ZERO, |e| p.max_leverage * e - v.foreign_gross_usd);
        assert!(gross <= budget.max(kept_gross), "case {case}: leverage");
        // Neutrality: inside the bound, or no more net exposure than the holds had.
        let (l, s) = books(&out.iter().map(|t| t.notional_usd).collect::<Vec<_>>());
        let (kl, ks) = books(&kept);
        assert!(
            imbalance(l, s) <= p.max_net_pct || (l - s).abs() <= (kl - ks).abs(),
            "case {case}: net {} vs bound {} and held net {}",
            (l - s).abs(),
            p.max_net_pct,
            (kl - ks).abs()
        );
    }
}

#[test]
fn deciding_twice_on_the_result_is_stable() {
    // Feed the targets back as holdings: nothing just opened is reversed, exited or shrunk.
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    for case in 0..1500 {
        let mut v = random_view(&mut rng);
        v.equity = Some(Decimal::from(100_000));
        let first = decide(&v);
        for (s, t) in v.symbols.iter_mut().zip(&first) {
            s.held_usd = t.notional_usd;
        }
        let second = decide(&v);
        for ((s, a), b) in v.symbols.iter().zip(&first).zip(&second) {
            if !a.notional_usd.is_zero() {
                assert_eq!(
                    Leg::of_notional(b.notional_usd),
                    Leg::of_notional(a.notional_usd),
                    "case {case}: {} reversed or exited",
                    s.symbol
                );
                assert!(
                    b.notional_usd.abs() >= a.notional_usd.abs(),
                    "case {case}: {} shrank",
                    s.symbol
                );
            }
        }
    }
}
