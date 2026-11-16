//! The trend filter's warm-up on past candles: three of its slow windows of 1m closes, folded into
//! its EMAs before the start so a market already trending into it holds the opening orders back
//! from the first placement. Without them the filter starts at the start's mid, as before.

use bb_strategy_sdk::CandleInterval;
use bb_strategy_sdk::{HistoryNeed, MAX_HISTORY_CANDLES, Warmup};

use super::GridStrategy;
use crate::plan::History;

impl GridStrategy {
    /// Three slow windows of 1m candles when the trend filter is on, all of them needed: a filter
    /// warmed on fewer would read a trend from part of one.
    pub(crate) fn history_need(&self) -> Option<HistoryNeed> {
        let cfg = self.config.trend_filter.as_ref()?;
        let bars =
            cfg.slow_secs.saturating_mul(3).div_ceil(60).clamp(1, u64::from(MAX_HISTORY_CANDLES));
        let bars = u32::try_from(bars).unwrap_or(1);
        Some(HistoryNeed { interval: CandleInterval::M1, want: bars, need: bars })
    }

    /// The trend filter's need, and for a derived range with no history from the build the daily
    /// candles over its lookback and the hourly ones for the ATR, all of them needed.
    pub(crate) fn history_needs(&self) -> Vec<HistoryNeed> {
        let mut needs: Vec<_> = self.history_need().into_iter().collect();
        if self.history.is_none() && self.config.needs_history() {
            let (days, hours) = self.config.history_bars();
            let all = |interval, n| HistoryNeed { interval, want: n, need: n };
            needs.extend([all(CandleInterval::D1, days), all(CandleInterval::H1, hours)]);
        }
        needs
    }

    /// The candles of one need: the range's daily and hourly ones, else the trend filter's.
    pub(crate) fn warm_for(&mut self, need: HistoryNeed, warmup: Warmup) {
        if !matches!(need.interval, CandleInterval::D1 | CandleInterval::H1) {
            return self.warm(warmup);
        }
        match warmup {
            Warmup::Loaded { candles, .. } => {
                let history = self.history.get_or_insert_with(History::default);
                if need.interval == CandleInterval::D1 {
                    history.daily = candles;
                } else {
                    history.hourly = candles;
                }
            }
            Warmup::Unavailable(why) => self.history_gap = Some(why),
        }
    }

    /// The warm-up's note stays up while the filter is young (three slow windows of live data
    /// from the start); by then the filter is warm either way.
    pub(crate) fn age_note(&mut self, now_ms: u64) {
        let (Some(cfg), Some(_)) = (&self.config.trend_filter, &self.warmup_note) else { return };
        let since = *self.note_since.get_or_insert(now_ms);
        if now_ms.saturating_sub(since) >= cfg.slow_secs.saturating_mul(3_000) {
            self.warmup_note = None;
        }
    }

    pub(crate) fn warm(&mut self, warmup: Warmup) {
        if self.config.trend_filter.is_none() {
            return;
        }
        let step = CandleInterval::M1.ms();
        self.warmup_note = Some(match warmup {
            Warmup::Loaded { candles, source, .. } => {
                let closes: Vec<_> =
                    candles.iter().map(|c| (c.open_ms.saturating_add(step), c.close)).collect();
                self.state.warm_trend(&closes);
                format!(
                    "Its trend filter warmed up on {} past 1m candles from {source}. ",
                    candles.len()
                )
            }
            Warmup::Unavailable(why) => format!(
                "Its trend filter couldn't load past prices ({why}), so it starts from the live \
                 price. "
            ),
        });
    }
}
