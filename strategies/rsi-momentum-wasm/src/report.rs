//! [`RsiReport`]: what the strategy shows of itself, as the built-in's report (same names, same
//! headline). Position, P&L, orders and the daily loss stop are the bot's.

use bb_strategy_sdk::{Decimal, ReportField, StrategyReport};
use bb_strategy_support::{MAX_REPORT_TEXT, clip};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RsiReport {
    pub symbol: String,
    pub interval: String,
    pub rsi: Option<Decimal>,
    /// Closed candles still needed before the RSI reads (and then one more for a cross).
    pub closes_to_warm: u32,
    pub mark: Option<Decimal>,
    pub cooldown_candles_left: u32,
    /// A close is under way: its attempts so far.
    pub close_attempts: Option<u32>,
    /// A close has failed past its attempts and is still being retried every tick.
    pub close_failing: bool,
    /// How the warm-up goes: a plugin always warms up from live data.
    pub history: Option<String>,
}

impl StrategyReport for RsiReport {
    fn headline(&self) -> String {
        clip(&self.headline_text(), MAX_REPORT_TEXT)
    }

    fn fields(&self) -> Vec<ReportField> {
        self.field_list()
    }

    /// The warm-up's note while the RSI is still warming up.
    fn briefing_lines(&self) -> Vec<String> {
        self.history.iter().filter(|_| self.closes_to_warm > 0).cloned().collect()
    }
}

impl RsiReport {
    fn headline_text(&self) -> String {
        if self.close_failing {
            return format!("{}: a close keeps failing, retrying every tick", self.symbol);
        }
        match (self.rsi, self.closes_to_warm) {
            (Some(rsi), _) => format!("{} RSI {rsi} on {} candles", self.symbol, self.interval),
            (None, n) => format!(
                "{}: warming up, {n} more {} candles for the first RSI",
                self.symbol, self.interval
            ),
        }
    }

    fn field_list(&self) -> Vec<ReportField> {
        let mut fields = vec![
            ReportField::text("symbol", clip(&self.symbol, MAX_REPORT_TEXT)),
            ReportField::text("interval", self.interval.clone()),
        ];
        fields.extend(self.rsi.map(|v| ReportField::number("rsi", v)));
        fields.push(ReportField::number("closes_to_warm", self.closes_to_warm.into()));
        fields.extend(self.mark.map(|v| ReportField::number("mark", v)));
        fields
            .push(ReportField::number("cooldown_candles_left", self.cooldown_candles_left.into()));
        fields.extend(self.close_attempts.map(|n| ReportField::number("close_attempts", n.into())));
        fields.push(ReportField::flag("close_failing", self.close_failing));
        fields.extend(
            self.history.iter().map(|h| ReportField::text("history", clip(h, MAX_REPORT_TEXT))),
        );
        fields
    }
}
