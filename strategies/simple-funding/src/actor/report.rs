//! What the actor reports after every tick, for the app to publish as the instance's snapshot: its
//! gross exposure and the day's P&L, so the account's caps count it. Read-only: nothing here
//! decides or trades.

use std::sync::Arc;

use rust_decimal::Decimal;

use super::SimpleFundingActor;

/// One tick's report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleFundingReport {
    /// Every universe position at its valuation (the live mark, else the venue's), USD.
    pub gross_usd: Decimal,
    /// The account's equity against the day's first read; `None` while either is unread.
    pub day_pnl_usd: Option<Decimal>,
}

/// Where the actor publishes its [`SimpleFundingReport`] after every tick.
pub trait ReportSink: Send + Sync {
    fn publish(&self, report: &SimpleFundingReport);
}

impl SimpleFundingActor {
    /// Publish a [`SimpleFundingReport`] to `sink` after every tick.
    #[must_use]
    pub fn with_report_sink(mut self, sink: Arc<dyn ReportSink>) -> Self {
        self.report_sink = Some(sink);
        self
    }

    pub(super) fn publish_report(&self) {
        let Some(sink) = &self.report_sink else { return };
        if !self.read_once {
            return;
        }
        let day_pnl_usd =
            self.equity.zip(self.day).and_then(|(e, d)| e.checked_sub(d.start_equity));
        sink.publish(&SimpleFundingReport { gross_usd: self.gross_usd, day_pnl_usd });
    }
}

/// What the strategy shows behind the host: its `/status` view (halted, entries blocked, unwinds,
/// pending commits, positions, signals), as the actor has always shown it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(transparent)]
pub struct SimpleFundingStatus {
    pub status: serde_json::Value,
}

impl bb_strategy_sdk::StrategyReport for SimpleFundingStatus {
    bb_strategy_sdk::typed_report!();
    fn headline(&self) -> String {
        let s = &self.status;
        let flag = |k: &str| s.get(k).and_then(serde_json::Value::as_bool).unwrap_or(false);
        let markets = s.get("markets").and_then(serde_json::Value::as_u64).unwrap_or(0);
        // Behind the host its own loss stop is off (the host's is the daily loss stop): a halt is
        // the margin halt.
        if flag("halted") && !flag("own_loss_stop") {
            format!("halted (margin); {markets} markets")
        } else if flag("halted") {
            format!("halted (daily loss or margin); {markets} markets")
        } else if flag("entries_blocked") {
            format!("entries wait (an unwind or an unsettled order); {markets} markets")
        } else {
            format!("carrying across {markets} markets")
        }
    }

    fn schema(&self) -> schemars::Schema {
        schemars::schema_for!(serde_json::Value)
    }
}
