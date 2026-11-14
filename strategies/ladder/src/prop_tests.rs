//! The property test: thousands of seeded random event sequences against a model venue (delays,
//! late trades, refused and unanswered cancels, sweeps, gaps), with the invariants of the ladder's
//! accounting checked as it goes and once everything has drained.

use bb_strategy_sdk::OrderType;

use crate::LadderSettings;
use crate::prop_model::Model;

fn settings() -> LadderSettings {
    LadderSettings {
        levels: 2,
        spacing_bps: "10".parse().unwrap(),
        order_size: "0.01".parse().unwrap(),
        order_type: OrderType::PostOnly,
    }
}

fn run(seed: u64) {
    let mut model = Model::new(seed, settings());
    model.start();
    for i in 0..200 {
        model.late = i >= 150;
        model.step();
    }
    model.drain();
    model.check_final();
}

/// Every seed must hold; a failure names the seed (re-run it alone with `replay`) and the last
/// events of its sequence.
#[test]
fn random_sequences_keep_the_ladders_accounting() {
    let mut failed = Vec::new();
    let mut first = String::new();
    for seed in 0..3_000 {
        if let Err(e) = std::panic::catch_unwind(|| run(seed)) {
            if first.is_empty() {
                first = e.downcast_ref::<String>().cloned().unwrap_or_default();
            }
            failed.push(seed);
        }
    }
    assert!(
        failed.is_empty(),
        "{} of 3000 seeds failed, e.g. {:?}\nfirst: {first}",
        failed.len(),
        failed.iter().take(5).collect::<Vec<_>>()
    );
}

/// One seed alone, for debugging: `replay(2)`.
#[test]
fn a_single_seed_replays() {
    run(std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1));
}
