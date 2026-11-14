//! The grid's cancel-all and what a failure of it means: the orders may still rest, so nothing is
//! placed until a cancel-all lands, and it is asked again on every tick. The built-in also tells
//! the owner after [`ALERT_AFTER`] failures in a row (`Intent::Notify`); the world has no alerts,
//! so the report's `cancel_failures` says it instead.

use bb_strategy_sdk::Intent;

/// Consecutive failed cancel-alls after which the built-in tells the owner.
pub const ALERT_AFTER: u32 = 5;

#[derive(Debug, Default)]
pub struct Pull {
    failed: bool,
    failures: u32,
}

impl Pull {
    /// A cancel-all failed and none has landed since: nothing may be placed.
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// Failed cancel-alls in a row.
    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// The cancel-all was answered (`None`: it landed) or failed (`Some(why)`). The built-in
    /// returns its alert here; the port has none to return.
    pub fn answered(&mut self, failure: Option<&str>) -> Option<Intent> {
        if failure.is_none() {
            self.failed = false;
            self.failures = 0;
        } else {
            self.failed = true;
            self.failures = self.failures.saturating_add(1);
        }
        None
    }
}
