//! A regression pin: every call rsi-momentum's own tests made to it under the bot's real host
//! (`parity/fixtures/rsi-momentum/`), replayed on this crate; the intents must be what they were.

use std::path::Path;

use bb_strategy_rsi_momentum::{RsiMomentum, RsiMomentumConfig};
use bb_strategy_sdk::{StrategyError, ValidateConfig};

fn build(settings: &str) -> Result<RsiMomentum, StrategyError> {
    let config: RsiMomentumConfig = serde_json::from_str(settings)
        .map_err(|e| StrategyError(format!("rsi-momentum settings: {e}")))?;
    config.validate().map_err(StrategyError)?;
    Ok(RsiMomentum::new(config))
}

#[test]
fn rsi_momentum_answers_every_recorded_call_as_it_did_under_the_host() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/rsi-momentum");
    let reports = bb_strategy_parity::replay_all(&dir, build).unwrap_or_else(|e| panic!("{e}"));
    let intents: usize = reports.iter().map(|r| r.intents).sum();
    eprintln!("{} fixtures replayed, {intents} intents compared", reports.len());
    assert!(intents > 0, "no intents compared");
}
