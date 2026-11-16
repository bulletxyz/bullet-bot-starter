//! What the strategy keeps across a restart: the UTC day's start equity, whether a stop has halted
//! it, and the symbols it opened. Saved as one [`bb_strategy_sdk::Persist`] write under
//! [`KEY_DAY_STATE`] (the bot's store keeps it in `day_state.json`, this same shape) and handed
//! back at the start.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// The write's key: the bot's store knows it as the day state.
pub const KEY_DAY_STATE: &str = "day_state";

/// The record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayState {
    /// UTC day index (unix ms / 86 400 000) of `start_equity`.
    pub day: u64,
    /// Account equity at the day's first read.
    pub start_equity: Decimal,
    /// The daily-loss (or margin) stop is holding the strategy flat.
    pub halted: bool,
    /// Symbols the strategy placed an opening order on and has not seen flat since: the positions
    /// it owns. A position outside this set at a live start is someone else's.
    #[serde(default)]
    pub owned: Vec<String>,
}
