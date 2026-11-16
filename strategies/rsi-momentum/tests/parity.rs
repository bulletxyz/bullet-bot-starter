//! A regression pin: every call rsi-momentum's own tests made to it under the bot's real host
//! (`parity/fixtures/rsi-momentum/`), replayed on this crate; the intents must be what they were.

use std::path::Path;

use bb_strategy_rsi_momentum::RsiMomentum;
use bb_strategy_sdk::{FromSettings, StrategyError};

/// The bot's build: the strategy from its settings, through the port.
fn build(settings: &str) -> Result<RsiMomentum, StrategyError> {
    RsiMomentum::from_settings(settings)
}

#[test]
fn rsi_momentum_answers_every_recorded_call_as_it_did_under_the_host() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/rsi-momentum");
    let reports = bb_strategy_parity::replay_all(&dir, build).unwrap_or_else(|e| panic!("{e}"));
    let intents: usize = reports.iter().map(|r| r.intents).sum();
    eprintln!("{} fixtures replayed, {intents} intents compared", reports.len());
    assert!(intents > 0, "no intents compared");
}
