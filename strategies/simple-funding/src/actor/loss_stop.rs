use std::sync::atomic::Ordering;

use rust_decimal::Decimal;

use super::{DAY_MS, Day, LossStop, SimpleFundingActor};
use crate::actor::day::{DayState, KEY_DAY_STATE};

impl SimpleFundingActor {
    // -- day, loss stop ---------------------------------------------------------------

    /// Track the UTC day and the halt: a new day or an owner resume lifts it and restarts
    /// the allowance from the current equity.
    pub(super) fn roll_day(&mut self, unix_ms: u64, equity: Option<Decimal>) {
        let index = unix_ms / DAY_MS;
        let resume = self.controls.resume.swap(false, Ordering::SeqCst);
        let Some(equity) = equity else {
            if resume && self.halted() {
                tracing::warn!("simple-funding: resume ignored, equity unknown");
                self.controls.resume.store(true, Ordering::SeqCst);
            }
            return;
        };
        // Ordered, never merely different: a clock that steps back, or a saved day ahead of
        // this clock, is not a new day and must not reset the baseline or lift the halt.
        let new_day = self.day.is_none_or(|d| index > d.index);
        if new_day || (resume && self.halted()) {
            if self.halted() {
                tracing::info!(resume, new_day, "simple-funding: daily-loss halt lifted");
                self.controls.halted.store(false, Ordering::SeqCst);
            }
            // The day key never moves backwards, even on an owner resume.
            let index = self.day.map_or(index, |d| d.index.max(index));
            self.day = Some(Day { index, start_equity: equity });
            self.persist();
        }
    }

    /// Halt when equity has dropped `daily_loss_usd` since the day start.
    pub(super) fn check_loss(&mut self, equity: Option<Decimal>) {
        let (Some(day), Some(equity)) = (self.day, equity) else { return };
        if self.loss_stop == LossStop::Host {
            return;
        }
        let drop = day.start_equity - equity;
        if !self.halted() && drop >= self.config.loss_stop_usd() {
            tracing::error!(
                %drop, limit = %self.config.loss_stop_usd(),
                "simple-funding: daily loss stop, flattening and halting"
            );
            self.halt();
        }
    }

    /// Hold the bot flat until the next UTC day or an owner resume, and remember it.
    pub(super) fn halt(&mut self) {
        if !self.controls.halted.swap(true, Ordering::SeqCst) {
            self.persist();
        }
    }

    /// The record to save now: the day, the halt and the owned symbols. `None` with no day yet.
    fn day_record(&self) -> Option<serde_json::Value> {
        let day = self.day?;
        let state = DayState {
            day: day.index,
            start_equity: day.start_equity,
            halted: self.halted(),
            owned: self.owned.iter().cloned().collect(),
        };
        serde_json::to_value(&state).ok()
    }

    /// Save the day, the halt and the owned symbols (nothing waits for the write).
    pub(super) fn persist(&self) {
        if !self.persists {
            return;
        }
        if let Some(record) = self.day_record() {
            self.io.persist(KEY_DAY_STATE, record);
        }
    }

    /// Record `symbol` as opened by this bot before the order goes out, so a crash after the
    /// fill still knows the position is its own. `false` when that cannot be made durable.
    pub(super) async fn claim(&mut self, symbol: &str) -> bool {
        if self.owned.contains(symbol) {
            return true;
        }
        self.owned.insert(symbol.to_string());
        if !self.persists {
            return true;
        }
        // The host applies the write before anything else of its round, then says so.
        let saved = match self.day_record() {
            Some(record) => self.io.persist_acked(KEY_DAY_STATE, record).await,
            None => Err("no day yet".to_string()),
        };
        match saved {
            Ok(()) => true,
            Err(e) => {
                tracing::error!(error = %e, "simple-funding: day state not saved");
                self.owned.remove(symbol);
                false
            }
        }
    }
}
