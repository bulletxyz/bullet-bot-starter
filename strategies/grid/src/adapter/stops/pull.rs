//! The stop's cancel (`CancelOwn` for the hard stop, `CancelAll` for the daily loss stop), asked again on every
//! tick until one lands; after [`ALERT_AFTER`] failures in a row the owner is told.

use bb_strategy_sdk::{Alert, Intent};

/// Consecutive failed cancel-alls before the owner is told (the grid keeps trying after it).
pub const ALERT_AFTER: u32 = 5;

/// The alert's key.
pub const KEY: &str = "grid_cancel_all_failing";

#[derive(Debug, Default)]
pub struct Pull {
    failures: u32,
    raised: bool,
}

impl Pull {
    /// A cancel-all failed and none has landed since: nothing may be placed.
    pub fn failed(&self) -> bool {
        self.failures > 0
    }

    /// The cancel-all was answered (`None`: it landed) or failed (`Some(why)`). Returns the alert
    /// to raise or resolve, if any.
    pub fn answered(&mut self, failure: Option<&str>) -> Option<Intent> {
        let Some(why) = failure else {
            self.failures = 0;
            return std::mem::take(&mut self.raised)
                .then(|| Intent::Notify(Alert::Resolve { key: KEY.to_string() }));
        };
        self.failures = self.failures.saturating_add(1);
        if self.failures < ALERT_AFTER || self.raised {
            return None;
        }
        self.raised = true;
        let text = format!(
            "cancelling the grid's orders has failed {} times in a row ({why}); they may still rest on \
             the venue. The grid keeps trying and places nothing until it lands.",
            self.failures
        );
        Some(Intent::Notify(Alert::Raise { key: KEY.to_string(), text }))
    }
}
