//! [`GridReport`]: what the grid shows of itself, as the built-in's report (the same names and
//! headline): its plan (range, step, size, stops, worst case), its levels, whether it paused or
//! stopped, and the P&L of its whole position. Orders and the position itself are the bot's.

use bb_strategy_sdk::{Decimal, ReportField, StrategyReport};
use bb_strategy_support::{MAX_REPORT_TEXT, clip, side_word};

use crate::grid::GridLevel;

/// The grid's own state. The plan's fields are `None` until the start resolved it.
#[derive(Debug, Clone)]
pub struct GridReport {
    /// `neutral`, `long` or `short`.
    pub direction: Option<String>,
    /// The range preset: `tight`, `normal` or `wide`.
    pub range: String,
    pub lower_price: Option<Decimal>,
    pub upper_price: Option<Decimal>,
    /// The price the levels were laid out around.
    pub anchor: Option<Decimal>,
    pub spacing: Option<Decimal>,
    pub order_size: Option<Decimal>,
    /// The hard stop below the range, when the grid holds a long side.
    pub stop_below: Option<Decimal>,
    /// The hard stop above the range, when the grid holds a short side.
    pub stop_above: Option<Decimal>,
    /// What the position costs with every losing-side level filled (USD).
    pub worst_case_usd: Option<Decimal>,
    /// The dollars it was sized from.
    pub amount_usd: Option<Decimal>,
    pub active_levels: usize,
    pub total_levels: usize,
    /// Its opening orders are held back: by the trend filter, or out of range on the profitable
    /// side.
    pub paused: bool,
    /// Why it stopped trading (the hard stop), when it did.
    pub stopped: Option<String>,
    /// Failed cancel-alls in a row (the built-in alerts the owner at five).
    pub cancel_failures: u32,
    pub pnl: GridPnl,
    pub levels: Vec<GridLevel>,
}

/// The whole position's P&L, as the host recorded it, in USD.
#[derive(Debug, Clone, Default)]
pub struct GridPnl {
    pub realized: Decimal,
    pub unrealized: Decimal,
    pub total: Decimal,
}

impl StrategyReport for GridReport {
    fn headline(&self) -> String {
        clip(&self.headline_text(), MAX_REPORT_TEXT)
    }

    fn fields(&self) -> Vec<ReportField> {
        self.field_list()
    }
}

impl GridReport {
    fn headline_text(&self) -> String {
        if let Some(why) = &self.stopped {
            return format!("stopped: {why}");
        }
        let (Some(lower), Some(upper)) = (self.lower_price, self.upper_price) else {
            return "not started".into();
        };
        let paused = if self.paused { ", its opening orders paused" } else { "" };
        format!(
            "{} of {} levels resting between {} and {}{paused}",
            self.active_levels,
            self.total_levels,
            lower.normalize(),
            upper.normalize()
        )
    }

    fn field_list(&self) -> Vec<ReportField> {
        let count = |n: usize| Decimal::from(u64::try_from(n).unwrap_or(u64::MAX));
        let mut f = Vec::new();
        f.extend(self.direction.clone().map(|d| ReportField::text("direction", d)));
        f.push(ReportField::text("range", self.range.clone()));
        let prices = [
            ("lower_price", self.lower_price),
            ("upper_price", self.upper_price),
            ("anchor", self.anchor),
            ("spacing", self.spacing),
            ("order_size", self.order_size),
            ("stop_below", self.stop_below),
            ("stop_above", self.stop_above),
        ];
        f.extend(prices.into_iter().filter_map(|(n, v)| Some(ReportField::number(n, v?))));
        f.extend(self.worst_case_usd.map(|v| ReportField::number("worst_case_usd", v).unit("USD")));
        f.extend(self.amount_usd.map(|v| ReportField::number("amount_usd", v).unit("USD")));
        f.push(ReportField::number("active_levels", count(self.active_levels)));
        f.push(ReportField::number("total_levels", count(self.total_levels)));
        f.push(ReportField::flag("paused", self.paused));
        f.extend(
            self.stopped.clone().map(|s| ReportField::text("stopped", clip(&s, MAX_REPORT_TEXT))),
        );
        f.push(ReportField::number("cancel_failures", self.cancel_failures.into()));
        f.push(ReportField::number("pnl.realized", self.pnl.realized).unit("USD"));
        f.push(ReportField::number("pnl.unrealized", self.pnl.unrealized).unit("USD"));
        f.push(ReportField::number("pnl.total", self.pnl.total).unit("USD"));
        // One field a level (at most 100, inside the bot's 128), e.g. `level 3`: `buy 101.5 Active`.
        for l in &self.levels {
            let side = l.side.map_or("-", side_word);
            let text = format!("{side} {} {:?}", l.price.normalize(), l.state);
            f.push(ReportField::text(format!("level {}", l.index), text));
        }
        f
    }
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
