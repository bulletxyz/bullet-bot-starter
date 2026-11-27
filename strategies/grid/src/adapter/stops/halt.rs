use bb_strategy_sdk::stops::StopReason;
use bb_strategy_sdk::{ExchangeIntent, Intent, NewOrder, OrderBook, OrderType, Side, View};
use bb_strategy_support::ioc::{MAX_IOC_BPS_FROM_MID, capped_ioc_price, two_sided_mid};
use bb_strategy_support::touch;
use rust_decimal::Decimal;

use crate::adapter::{CLOSE_SLIP_BPS, GridStrategy, Halt, KEY_HARD_STOP};

impl GridStrategy {
    /// The reason to stop now, if any, and the cancel it asks. The hard stop past the range is the
    /// grid's own stop, a normal one: its state is trusted and it ends its own trading, so only its
    /// own orders go ([`ExchangeIntent::CancelOwn`]; the owner's on the market stay). The daily loss
    /// stop is a safety stop: every order on the market goes ([`ExchangeIntent::CancelAll`]), as the path's
    /// halt for it: the host's (`View::stop`), or the build's closure.
    fn stop_reason(
        &self,
        mid: Option<Decimal>,
        host: Option<&StopReason>,
    ) -> Option<(String, Intent)> {
        let plan = self.plan.as_ref()?;
        if let Some(mid) = mid {
            if let Some(stop) = plan.stop_below
                && mid <= stop
            {
                let why = format!("hard stop: {} at {mid}, at or below {stop}", self.symbol());
                return Some((why, self.cancel_own()));
            }
            if let Some(stop) = plan.stop_above
                && mid >= stop
            {
                let why = format!("hard stop: {} at {mid}, at or above {stop}", self.symbol());
                return Some((why, self.cancel_own()));
            }
        }
        let day_stop = matches!(host, Some(StopReason::DailyLoss { .. }));
        if day_stop || self.day_stop.as_ref().is_some_and(|tripped| tripped()) {
            return Some(("daily loss stop".into(), self.cancel_all()));
        }
        None
    }

    /// A reduce-only IOC closing the market's position, capped around the mid; none when flat or with
    /// no two-sided book. The order path holds it to the venue's position.
    pub(in crate::adapter) fn close(&mut self, view: &View<'_>, attempt: u32) -> Option<Intent> {
        let position = self.position(view);
        if position.is_zero() {
            return None;
        }
        let book = self.book(view)?;
        let mid = two_sided_mid(book)?;
        let side = if position > Decimal::ZERO { Side::Sell } else { Side::Buy };
        let touch = touch(book, side)?;
        let slip = CLOSE_SLIP_BPS * Decimal::from(attempt.max(1));
        let price = capped_ioc_price(side, touch, mid, slip, MAX_IOC_BPS_FROM_MID)?;
        Some(Self::on_bullet(ExchangeIntent::Place(NewOrder {
            protect: None,
            symbol: self.symbol().to_string(),
            side,
            order_type: OrderType::Market,
            price,
            quantity: position.abs(),
            client_id: Some(self.client_ids.issue()),
            reduce_only: true,
        })))
    }

    /// Rule 2 and the daily loss stop: cancel, close, stop placing. Once stopped, a tick
    /// closes again while the position is open.
    pub(in crate::adapter) fn check_halt(
        &mut self,
        view: &View<'_>,
        tick: bool,
    ) -> Option<Vec<Intent>> {
        let position = self.position(view);
        if let Some(halt) = &mut self.halt {
            if !tick || position.is_zero() {
                return Some(Vec::new());
            }
            halt.attempts = halt.attempts.saturating_add(1);
            let attempt = halt.attempts;
            tracing::warn!(attempt, %position, "Grid stopped — closing again");
            return Some(self.close(view, attempt).into_iter().collect());
        }
        let mid = self.book(view).and_then(OrderBook::midpoint);
        let (reason, pull) = self.stop_reason(mid, view.stop)?;
        tracing::warn!(%reason, %position, "Grid stopping: cancel and close");
        self.state.clear_all();
        // Only the hard stop pulls its own orders; the loss watch holds a daily loss trip.
        let hard = matches!(pull, Intent::On { intent: ExchangeIntent::CancelOwn(_), .. });
        let said = if hard { self.hold_for_hard_stop(&reason) } else { Vec::new() };
        self.halt = Some(Halt { reason, attempts: 1, pull: pull.clone() });
        let mut out = vec![pull];
        out.extend(self.close(view, 1));
        out.extend(said);
        Some(out)
    }

    /// Hold the instance for the hard stop (the build's hold, else the host's stand-down, after the
    /// close), and what the owner is told of it.
    fn hold_for_hard_stop(&self, reason: &str) -> Vec<Intent> {
        tracing::warn!(%reason, "Grid hard stop: holding the instance until the owner resumes");
        let mut out = Vec::new();
        match &self.hard_stop_hold {
            Some(hold) => hold(),
            None => out.push(Intent::StandDown(format!("grid {reason}"))),
        }
        let text = format!(
            "Grid {reason}: it closed its position and stays held, through a restart too, until you \
             resume it. A resume starts a new grid at the price then."
        );
        out.push(Intent::Notify(bb_strategy_sdk::Alert::Raise {
            key: KEY_HARD_STOP.to_string(),
            text,
        }));
        out
    }

    /// While the stop's cancel has not landed orders may still rest, so it is asked again, the same kind.
    pub(in crate::adapter) fn ask_pull_again(&self, intents: &mut Vec<Intent>) {
        let pulls = |i: &Intent| {
            matches!(
                i,
                Intent::On {
                    intent: ExchangeIntent::CancelAll(_) | ExchangeIntent::CancelOwn(_),
                    ..
                }
            )
        };
        if let Some(halt) = &self.halt
            && self.pull.failed()
            && !intents.iter().any(pulls)
        {
            intents.push(halt.pull.clone());
        }
    }
}
