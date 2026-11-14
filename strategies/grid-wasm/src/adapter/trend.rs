//! The pauses: the trend filter holds the opening orders back when the market runs away, and the
//! range holds them back out of range on the profitable side. The closing orders stay.

use bb_strategy_sdk::{Decimal, Intent, OrderBook, View};

use super::GridStrategy;
use crate::derive::Direction;

impl GridStrategy {
    /// The trend filter: past its divergence the opening orders are cancelled, the closing ones
    /// stay; when it clears they come back.
    pub(crate) fn handle_trend_filter(&mut self, view: &View<'_>) -> Vec<Intent> {
        let Some(cfg) = self.config.trend_filter.clone() else {
            return Vec::new();
        };
        let Some(mid) = view.book.and_then(OrderBook::midpoint) else {
            return Vec::new();
        };
        let was_paused = self.state.paused;
        let (_divergence, paused) = self.state.update_trend_filter(mid, view.now_ms, &cfg);
        if paused && !was_paused {
            return self.pause_opening(view.position);
        }
        if !paused && was_paused && !self.range_paused {
            self.state.unpause_all();
        }
        Vec::new()
    }

    /// Rule 3: out of range on the profitable side (long above the range, short below it), the
    /// opening orders are cancelled until the price is back inside.
    pub(crate) fn handle_range(&mut self, mid: Decimal, position: Decimal) -> Vec<Intent> {
        let Some(plan) = &self.plan else { return Vec::new() };
        let out = match plan.direction {
            Direction::Neutral => false,
            Direction::Long => mid > plan.upper,
            Direction::Short => mid < plan.lower,
        };
        if out && !self.range_paused {
            self.range_paused = true;
            return self.pause_opening(position);
        }
        if !out && self.range_paused {
            self.range_paused = false;
            if !self.state.paused {
                self.state.unpause_all();
            }
        }
        Vec::new()
    }
}
