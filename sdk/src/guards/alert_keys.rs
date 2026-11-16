//! What every strategy's guards say to the user: the alert type and its stable keys, and the reject
//! breaker's and the reconcile cadence's defaults. The classifiers of venue error text are the
//! host's.

/// Consecutive rejected placements that trip the reject breaker.
pub const REJECTS_TO_TRIP: u32 = 5;
/// Cadence of the open-order reconciliation.
pub const RECONCILE_SECS: u64 = 60;

/// An alert for the user. `key` is stable so a sink can de-duplicate and
/// match a `Resolve` to its `Raise`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardAlert {
    Raise { key: &'static str, text: String },
    Resolve { key: &'static str },
}

impl From<GuardAlert> for crate::host::capability::Alert {
    fn from(alert: GuardAlert) -> Self {
        match alert {
            GuardAlert::Raise { key, text } => Self::Raise { key: key.to_string(), text },
            GuardAlert::Resolve { key } => Self::Resolve { key: key.to_string() },
        }
    }
}

pub const KEY_STALE_PRICE: &str = "stale_price";
pub const KEY_REJECTS: &str = "rejects";
pub const KEY_PARAMS_FILE: &str = "params_file";
pub const KEY_STOPS_FILE: &str = "stops_file";
pub const KEY_POSITION_RESYNC: &str = "position_resync";
pub const KEY_VENUE_STALE: &str = "venue_stale_price";
pub const KEY_DELEGATION_MOVED: &str = "delegation_moved";
/// The host could not write the strategy's state.
pub const KEY_STATE_WRITE: &str = "state_write";
/// The venue left a run of orders unanswered.
pub const KEY_UNKNOWN_OUTCOMES: &str = "unknown_outcomes";
/// A strategy cancelled faster than the cap allows: its instance is paused.
pub const KEY_CANCEL_RATE: &str = "cancel_rate";
/// The cancel after a run of unanswered orders did not go through.
pub const KEY_UNKNOWN_CANCEL: &str = "unknown_outcomes_cancel";
