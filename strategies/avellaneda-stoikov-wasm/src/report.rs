//! [`AsReport`]: what the strategy shows of itself, as the built-in's status shows it (position,
//! fills, quote slots, sigma and its sample count). Position and P&L proper are the bot's.

use bb_strategy_sdk::{Decimal, ReportField, StrategyReport};
use bb_strategy_support::{MAX_REPORT_TEXT, clip};

#[derive(Debug, Clone, PartialEq)]
pub struct AsReport {
    pub symbol: String,
    /// `single-venue` or `fair-value` (priced off the reference leg).
    pub mode: &'static str,
    pub net_position: Decimal,
    pub total_fills: u64,
    /// Quotes it believes rest on the venue.
    pub quotes: usize,
    pub best_bid: Option<Decimal>,
    pub best_ask: Option<Decimal>,
    pub mid: Option<Decimal>,
    /// The volatility estimate; `None` until three samples.
    pub sigma: Option<f64>,
    pub vol_samples: usize,
    /// Fair-value mode: the reference is stale, so nothing is requoted.
    pub reference_stale: bool,
}

impl StrategyReport for AsReport {
    fn headline(&self) -> String {
        let text = match (self.sigma, self.reference_stale) {
            (_, true) => format!("{}: the reference price is stale, not requoting", self.symbol),
            (None, _) => format!(
                "{}: waiting for price samples before the first quote ({} so far)",
                self.symbol, self.vol_samples
            ),
            (Some(_), _) => format!(
                "{}: {} quotes up, position {}",
                self.symbol, self.quotes, self.net_position
            ),
        };
        clip(&text, MAX_REPORT_TEXT)
    }

    fn fields(&self) -> Vec<ReportField> {
        let mut fields = vec![
            ReportField::text("symbol", clip(&self.symbol, MAX_REPORT_TEXT)),
            ReportField::text("mode", self.mode),
            ReportField::number("net_position", self.net_position),
            ReportField::number("total_fills", self.total_fills.into()),
            ReportField::number("quotes", u64::try_from(self.quotes).unwrap_or(u64::MAX).into()),
        ];
        fields.extend(self.best_bid.map(|v| ReportField::number("best_bid", v)));
        fields.extend(self.best_ask.map(|v| ReportField::number("best_ask", v)));
        fields.extend(self.mid.map(|v| ReportField::number("mid", v)));
        fields.extend(
            self.sigma
                .and_then(Decimal::from_f64_retain)
                .map(|v| ReportField::number("sigma", v.round_dp(12))),
        );
        fields.push(ReportField::number(
            "vol_samples",
            u64::try_from(self.vol_samples).unwrap_or(u64::MAX).into(),
        ));
        fields.push(ReportField::flag("reference_stale", self.reference_stale));
        fields
    }
}
