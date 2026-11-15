//! The pauses: the trend filter holds the opening orders back when the market runs away, and the
//! range holds them back out of range on the profitable side. The closing orders stay.

use bb_strategy_sdk::OrderBook;
use bb_strategy_sdk::{Intent, View};
use rust_decimal::Decimal;

use super::GridStrategy;
use crate::derive::Direction;
use crate::levels::LevelState;

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
        let (div, paused) = self.state.update_trend_filter(mid, view.now_ms, &cfg);
        if paused && !was_paused {
            tracing::warn!(
                divergence_bps = %format!("{div:.1}"),
                threshold_bps = %cfg.pause_divergence_bps,
                "Trend filter tripped — cancelling the grid's opening orders"
            );
            return self.pause_opening(view.position, view.now_ms);
        }
        if !paused && was_paused {
            tracing::info!(divergence_bps = %format!("{div:.1}"), "Trend filter cleared");
            self.resume_if_clear();
        }
        Vec::new()
    }

    /// Rule 3: out of range on the profitable side (long above the range, short below it), the
    /// opening orders are cancelled until the price is back inside.
    pub(crate) fn handle_range(
        &mut self,
        mid: Decimal,
        position: Decimal,
        now_ms: u64,
    ) -> Vec<Intent> {
        let Some(plan) = &self.plan else { return Vec::new() };
        let out = match plan.direction {
            Direction::Neutral => false,
            Direction::Long => mid > plan.upper,
            Direction::Short => mid < plan.lower,
        };
        if out && !self.range_paused {
            tracing::warn!(%mid, "Price left the range on the profitable side — pausing opening orders");
            self.range_paused = true;
            return self.pause_opening(position, now_ms);
        }
        if !out && self.range_paused {
            tracing::info!(%mid, "Price back in the range — resuming grid");
            self.range_paused = false;
            self.resume_if_clear();
        }
        Vec::new()
    }

    /// Hold every opening order back (closing ones stay): cancels for the resting ones, each order
    /// let go of until a read shows it gone.
    fn pause_opening(&mut self, position: Decimal, now_ms: u64) -> Vec<Intent> {
        let closing = self.state.closing(position, self.order_size());
        let mut out = Vec::new();
        for (i, closes) in closing.into_iter().enumerate() {
            let l = &self.state.levels[i];
            if closes || !matches!(l.state, LevelState::Active | LevelState::Pending) {
                continue;
            }
            if let Some(cancel) = self.cancel_of(i) {
                self.let_go_of(i, now_ms);
                out.push(cancel);
            }
            self.state.pause(i);
        }
        out
    }
}
