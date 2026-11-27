//! The commands the market maker answers itself: a live change of its params, a pause and a
//! resume (when the host leaves them to it), and a close: no quote from then on, the host's close
//! of the position, and the reply worded from what the host reports.

#![expect(clippy::arithmetic_side_effects, reason = "Decimal sizes far below 7.9e28")]

use bb_strategy_sdk::{
    BULLET, CloseReport, ControlCmd, ControlReply, Intent, SettingChanges, View,
};
use bb_strategy_support::floor_to_step;
use rust_decimal::Decimal;

use super::{MarketMaker, PULL_RETRY_MS};
use crate::params::SizeBounds;

/// A tightening of the limits is the host's: its order path holds them.
pub const LIMITS_ARE_THE_HOSTS: &str =
    "Not applied: the bot's host holds the limits, not the market maker.";

pub(crate) fn pause_reply(pulled: bool, net: Decimal) -> ControlReply {
    if pulled {
        ControlReply::Ok(format!("Paused. All quotes cancelled; position {net} kept."))
    } else {
        ControlReply::Ok(format!(
            "Paused, but cancelling open orders failed; retrying every {}s. Position {net} kept.",
            PULL_RETRY_MS / 1000
        ))
    }
}

impl MarketMaker {
    pub(crate) fn on_control(&mut self, id: u64, cmd: &ControlCmd, view: &View<'_>) -> Vec<Intent> {
        let reply = match cmd {
            ControlCmd::SetSettings { changes, .. } => self.set_settings(changes, view),
            ControlCmd::SetLimits(_) => ControlReply::Refused(LIMITS_ARE_THE_HOSTS.into()),
            ControlCmd::Pause => {
                if self.pull.pause_reply.is_some() {
                    ControlReply::Ok("Pausing already: the quotes are being cancelled.".into())
                } else {
                    self.paused = true;
                    self.pull.pause_reply = Some(id);
                    // The reply follows the pull's answer.
                    return self.pull_quotes(view, true);
                }
            }
            ControlCmd::Resume => {
                self.paused = false;
                self.forced = true;
                let bullet = Self::bullet(view);
                match bullet.and_then(|e| e.path_stop.as_deref().or(e.venue_halt.as_deref())) {
                    Some(why) => {
                        ControlReply::Ok(format!("Resumed, but new orders still stop: {why}."))
                    }
                    None => ControlReply::Ok(
                        "Resumed. Quoting restarts on the next book update.".to_string(),
                    ),
                }
            }
            ControlCmd::ClosePosition => {
                if self.closing.is_some() {
                    ControlReply::Refused(
                        "A close is already in progress; try again when it has finished.".into(),
                    )
                } else {
                    // No quote from now on; the host's close cancels what rests first.
                    self.closing = Some(id);
                    self.paused = true;
                    let (exchange, symbol) = (BULLET.to_string(), self.symbol().to_string());
                    return vec![Intent::ClosePosition { id, exchange, symbol }];
                }
            }
            ControlCmd::KillSwitch { .. } => ControlReply::Refused(
                "The kill switch is account-wide: the supervisor runs it, not one instance."
                    .to_string(),
            ),
            ControlCmd::Stage { .. } | ControlCmd::Unstage { .. } => {
                ControlReply::Refused("Staging is not available on this bot.".to_string())
            }
        };
        vec![Intent::Reply { id, reply }]
    }

    fn set_settings(&mut self, changes: &SettingChanges, view: &View<'_>) -> ControlReply {
        let size = SizeBounds::new(self.step, self.caps(view).max_position);
        match self.params.changed(changes, size) {
            Ok(next) => {
                let before = std::mem::replace(&mut self.params, next);
                self.pricing.on_params(&before, &self.params);
                ControlReply::Ok(format!(
                    "Applied {}. Takes effect on the next quote refresh.",
                    self.params.summary(&before, changes)
                ))
            }
            Err(e) => ControlReply::Refused(format!("Not applied: {e}.")),
        }
    }

    pub(crate) fn close_failed(&mut self, id: u64) -> Vec<Intent> {
        if self.closing.take_if(|c| *c == id).is_none() {
            return Vec::new();
        }
        let why = "The close could not be carried out; try again shortly.".to_string();
        vec![Intent::Reply { id, reply: ControlReply::Refused(why) }]
    }

    /// The host's close ended: the reply, from what it found and did.
    pub(crate) fn on_closed(&mut self, id: u64, report: &CloseReport) -> Vec<Intent> {
        if self.closing.take_if(|c| *c == id).is_none() {
            return Vec::new();
        }
        if report.cancel_error.is_none() {
            self.slots = [None, None];
            self.strays.clear();
        }
        vec![Intent::Reply { id, reply: self.close_reply(report) }]
    }

    fn close_reply(&self, report: &CloseReport) -> ControlReply {
        if report.cancel_error.is_some() {
            return ControlReply::Refused(
                "Paused, but cancelling open orders failed, so the position was not closed. Try \
                 again shortly."
                    .to_string(),
            );
        }
        let start = match &report.start {
            Ok(start) => *start,
            Err(e) => {
                return ControlReply::Refused(format!(
                    "Paused and pulled the quotes, but could not read the position, so nothing \
                     was closed: {e}"
                ));
            }
        };
        let step = self.step.filter(|s| *s > Decimal::ZERO);
        let lots = |x: Decimal| step.map_or(x, |s| floor_to_step(x, s));
        let symbol = self.symbol();
        let base = symbol.split('-').next().unwrap_or(symbol);
        // Flat (or below the venue's minimum): paused and pulled, no IOC.
        if lots(start.abs()).is_zero() {
            return ControlReply::Ok(if start.is_zero() {
                "Paused and pulled the quotes. The position is flat; nothing to close.".to_string()
            } else {
                format!(
                    "Paused and pulled the quotes. The position ({} {base}) is below the venue's \
                     minimum size; nothing to close.",
                    start.normalize()
                )
            });
        }
        let fmt = |x: Decimal| match step {
            Some(s) => format!("{:.*}", s.normalize().scale() as usize, x),
            None => x.normalize().to_string(),
        };
        let remaining = report.remaining;
        let (closed, left) = (fmt((start - remaining).abs()), fmt(remaining.abs()));
        let mut text = if lots(remaining.abs()).is_zero() && report.stopped_early.is_none() {
            format!("Closed {closed} {base}; {left} left. The bot is paused.")
        } else {
            let plural = if report.attempts == 1 { "" } else { "s" };
            format!(
                "{left} {base} still open after {} attempt{plural} (closed {closed} {base}). The \
                 bot is paused.",
                report.attempts
            )
        };
        if let Some(why) = &report.stopped_early {
            text = format!("{text} Stopped early: {why}.");
        }
        ControlReply::Ok(text)
    }
}
