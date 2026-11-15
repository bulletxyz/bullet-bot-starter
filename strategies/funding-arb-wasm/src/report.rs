//! [`ArbReport`]: what the plugin shows of itself. Position, P&L and orders are the bot's.

use bb_strategy_sdk::{Decimal, ReportField, StrategyReport};
use bb_strategy_support::{MAX_REPORT_TEXT, clip};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArbReport {
    pub symbol: String,
    pub hedge_symbol: Option<String>,
    pub signal_8h: Option<Decimal>,
    pub round_trip_cost: Option<Decimal>,
    pub target_usd: Option<Decimal>,
    /// `ShortBullet` or `LongBullet`, from the last decision.
    pub direction: Option<String>,
    pub bullet_base: Option<Decimal>,
    pub hl_base: Option<Decimal>,
    /// What the plugin is doing: `idle`, `reading`, `acting`.
    pub state: String,
    /// The last thing worth saying (a refusal, an unwind, a gate that blocked an entry).
    pub note: Option<String>,
    /// Consecutive rounds a close or unwind has been needed.
    pub unwind_attempts: u32,
}

impl StrategyReport for ArbReport {
    fn headline(&self) -> String {
        clip(&self.headline_text(), MAX_REPORT_TEXT)
    }

    fn fields(&self) -> Vec<ReportField> {
        let mut f = vec![
            ReportField::text("symbol", clip(&self.symbol, MAX_REPORT_TEXT)),
            ReportField::text("state", self.state.clone()),
        ];
        f.extend(
            self.hedge_symbol
                .iter()
                .map(|s| ReportField::text("hedge_symbol", clip(s, MAX_REPORT_TEXT))),
        );
        f.extend(self.signal_8h.map(|v| ReportField::number("signal_8h", v)));
        f.extend(self.round_trip_cost.map(|v| ReportField::number("round_trip_cost", v)));
        f.extend(self.target_usd.map(|v| ReportField::number("target_usd", v).unit("USD")));
        f.extend(self.direction.iter().map(|v| ReportField::text("direction", v.clone())));
        f.extend(self.bullet_base.map(|v| ReportField::number("bullet_base", v)));
        f.extend(self.hl_base.map(|v| ReportField::number("hl_base", v)));
        f.push(ReportField::number("unwind_attempts", self.unwind_attempts.into()));
        f.extend(self.note.iter().map(|n| ReportField::text("note", clip(n, MAX_REPORT_TEXT))));
        f
    }
}

impl ArbReport {
    fn headline_text(&self) -> String {
        if self.unwind_attempts > 0 {
            return format!("{}: unwinding a one-sided or mismatched pair", self.symbol);
        }
        match (self.signal_8h, self.target_usd) {
            (Some(sig), Some(t)) => {
                format!("{} funding spread {sig} per 8h, target {t} USD", self.symbol)
            }
            _ => format!("{}: waiting for funding history and both books", self.symbol),
        }
    }
}
