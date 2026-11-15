//! What the strategy shows of itself: the built-in's status (halted, entries blocked, unwinds,
//! pending commits, positions, signals) as report fields.

use std::collections::BTreeMap;

use bb_strategy_sdk::{Decimal, ReportField, StrategyReport};
use bb_strategy_support::{MAX_REPORT_TEXT, clip};

/// Most per-market fields shown (a report holds at most 128 fields).
const MAX_MARKET_FIELDS: usize = 50;

/// The state a flow leaves for the report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub halted: bool,
    pub entries_blocked: bool,
    /// Why the strategy does not trade at all (a start over positions it did not open).
    pub refused: Option<String>,
    /// Outstanding unwinds: symbol, text.
    pub unwinds: BTreeMap<String, String>,
    pub pending_commits: usize,
    pub markets: usize,
    pub equity: Option<Decimal>,
    pub day_start_equity: Option<Decimal>,
    pub gross_usd: Decimal,
    /// Signed base positions held in the universe.
    pub positions: BTreeMap<String, Decimal>,
    pub signals: BTreeMap<String, Decimal>,
}

impl StrategyReport for Status {
    fn headline(&self) -> String {
        let text = if let Some(why) = &self.refused {
            format!("not trading: {why}")
        } else if self.halted {
            format!("halted (daily loss or margin); {} markets", self.markets)
        } else if self.entries_blocked {
            format!("entries wait (an unwind or an unsettled order); {} markets", self.markets)
        } else {
            format!("carrying across {} markets", self.markets)
        };
        clip(&text, MAX_REPORT_TEXT)
    }

    fn fields(&self) -> Vec<ReportField> {
        let mut fields = vec![
            ReportField::flag("halted", self.halted),
            ReportField::flag("entries_blocked", self.entries_blocked),
            ReportField::number("markets", Decimal::from(self.markets as u64)),
            ReportField::number("pending_commits", Decimal::from(self.pending_commits as u64)),
            ReportField::number("gross", self.gross_usd).unit("USD"),
        ];
        fields.extend(
            self.refused.iter().map(|w| ReportField::text("refused", clip(w, MAX_REPORT_TEXT))),
        );
        fields.extend(self.equity.map(|v| ReportField::number("equity", v).unit("USD")));
        fields.extend(
            self.day_start_equity.map(|v| ReportField::number("day_start_equity", v).unit("USD")),
        );
        for (symbol, text) in &self.unwinds {
            fields.push(ReportField::text(
                clip(&format!("unwind_{symbol}"), 64),
                clip(text, MAX_REPORT_TEXT),
            ));
        }
        for (symbol, v) in self.positions.iter().take(MAX_MARKET_FIELDS) {
            fields.push(ReportField::number(clip(&format!("position_{symbol}"), 64), *v));
        }
        for (symbol, v) in self.signals.iter().take(MAX_MARKET_FIELDS) {
            fields.push(ReportField::number(clip(&format!("signal_{symbol}"), 64), v.round_dp(6)));
        }
        fields.truncate(128);
        fields
    }
}
