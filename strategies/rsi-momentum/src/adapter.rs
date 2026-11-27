//! RSI momentum as a [`StrategyAdapter`]: candles from the mark price, the RSI at each close, and
//! the entries, exits and stop it asks for as intents. The host owns everything around them: the
//! position (from fills, taken over from the venue when they disagree), the order path's limits,
//! the paper broker, the daily loss stop and its saved day, and the flatten on wind-down.
//!
//! Rules kept here (the strategy's own; the tests are in `adapter_tests.rs`):
//! - no trading during warm-up (`rsi_period + 1` closes) or after a gap in the data (the RSI warms
//!   up again); an entry needs a cross, a fresh mark, a live two-sided book near it, and is sized
//!   to `notional_usd` within `max_position`;
//! - every IOC is priced within [`MAX_IOC_BPS_FROM_MID`] of the book's mid (a close without a live
//!   book: of the mark, else the entry price);
//! - a close is reduce-only, sized to the venue's position as last read, else the host's, retried every tick
//!   with a wider cross; after [`CLOSE_ATTEMPTS`] it raises `rsi_momentum_close_failing` for the owner
//!   (`Intent::Notify`, resolved by the next start) and the closes go on;
//! - no entry while a close is under way, an entry is in flight, or a position is held.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal prices, sizes and bps far below 7.9e28; attempt counters bounded by CLOSE_ATTEMPTS"
)]

use bb_strategy_sdk::{
    Alert, BULLET, Event, ExchangeIntent, HistoryNeed, Intent, MAX_HISTORY_CANDLES, MarketEvent,
    MarketView, StrategyAdapter, StrategyError, View, Warmup,
};
use bb_strategy_sdk::{NewOrder, OrderType, Protection, Side};
use bb_strategy_support::ClientIdIssuer;
use bb_strategy_support::ioc::{capped_ioc_price, two_sided_mid, within_bps};
use bb_strategy_support::{close_reference, touch};
use rust_decimal::Decimal;

use crate::config::RsiMomentumConfig;
use crate::decide::{self, Intent as Signal};
use crate::indicator::candles::{CandleBuilder, Closed};
use crate::indicator::rsi::Rsi;
use crate::limits::{CLOSE_ATTEMPTS, CLOSE_SLIPPAGE_MULT, ENTRY_SETTLE_SECS, MAX_IOC_BPS_FROM_MID};
use crate::report::RsiReport;

/// The alert key a close that keeps failing is raised under.
pub const ALERT_CLOSE: &str = "rsi_momentum_close_failing";

/// A close under way: how many attempts it has made.
#[derive(Debug, Clone, Copy)]
struct Closing {
    attempts: u32,
}

pub struct RsiMomentum {
    config: RsiMomentumConfig,
    candles: CandleBuilder,
    rsi: Rsi,
    last_rsi: Option<Decimal>,
    /// The last mark and when it came (`View::now_ms`).
    mark: Option<(Decimal, u64)>,
    /// Closed candles left before an entry is allowed again.
    cooldown_left: u32,
    closing: Option<Closing>,
    /// An entry sent, not yet seen as a fill: when.
    entry_sent_ms: Option<u64>,
    /// The venue's own signed position as the last read showed it (start, a tick's read); `None`
    /// when unknown: a tick that did not read it, or a fill since.
    venue: Option<Decimal>,
    /// When the last book came (`View::now_ms` of its `Event::Book`, or the start's).
    book_ms: Option<u64>,
    client_ids: ClientIdIssuer,
    /// After a warm-up on past candles: the candle the first live mark must fall in, the one
    /// that opened as the last loaded one closed. Any other and the warm-up is dropped.
    seam: Option<u64>,
    /// What the warm-up came to, in plain words, for the briefing.
    history: Option<String>,
}

impl RsiMomentum {
    #[must_use]
    pub fn new(config: RsiMomentumConfig) -> Self {
        Self {
            candles: CandleBuilder::new(config.interval),
            rsi: Rsi::new(config.rsi_period),
            config,
            last_rsi: None,
            mark: None,
            cooldown_left: 0,
            closing: None,
            entry_sent_ms: None,
            venue: None,
            book_ms: None,
            client_ids: ClientIdIssuer::session_seeded(),
            seam: None,
            history: None,
        }
    }

    fn fresh_mark(&self, now_ms: u64) -> Option<Decimal> {
        let limit = self.config.stale_secs.saturating_mul(1_000);
        self.mark.filter(|(_, at)| now_ms.saturating_sub(*at) <= limit).map(|(m, _)| m)
    }

    /// Its market as the host shows it: `symbol` on Bullet.
    fn market<'v>(&self, view: &'v View<'_>) -> Option<&'v MarketView> {
        view.market(BULLET, &self.config.symbol)
    }

    /// The host's position in its market (zero when the market is not shown).
    fn position(&self, view: &View<'_>) -> Decimal {
        self.market(view).map_or(Decimal::ZERO, |m| m.position)
    }

    fn avg_entry(&self, view: &View<'_>) -> Option<Decimal> {
        self.market(view).and_then(|m| m.avg_entry)
    }

    /// Bullet's signed position in its market from the host's last read; `None` while unread.
    fn venue_read(&self, view: &View<'_>) -> Option<Decimal> {
        view.exchange(BULLET).and_then(|e| e.venue_position(&self.config.symbol))
    }

    /// The event if it is on its market (Bullet, `symbol`).
    fn ours<'e>(&self, event: &'e Event) -> Option<&'e MarketEvent> {
        match event {
            Event::Market { exchange, symbol, event }
                if exchange == BULLET && *symbol == self.config.symbol =>
            {
                Some(event)
            }
            _ => None,
        }
    }

    /// The venue's position as the host last read it (`ExchangeView::positions`: the start's and a
    /// tick's read), kept until a fill or a tick that did not read it; and when the last book came.
    fn observe(&mut self, event: &Event, view: &View<'_>) {
        let ours = self.ours(event);
        match (ours, self.venue_read(view)) {
            (_, Some(read)) => self.venue = Some(read),
            (Some(MarketEvent::Fill(_)), None) => self.venue = None,
            (None, None) if matches!(event, Event::Tick) => self.venue = None,
            _ => {}
        }
        if matches!(ours, Some(MarketEvent::Book)) {
            self.book_ms = Some(view.now_ms);
        }
    }

    /// The position a close is sized and ended on: the venue's as the host last read it, else the
    /// host's.
    fn held(&self, view: &View<'_>) -> Decimal {
        self.venue.unwrap_or_else(|| self.position(view))
    }

    fn held_side(&self, view: &View<'_>) -> Option<Side> {
        match self.position(view).cmp(&Decimal::ZERO) {
            std::cmp::Ordering::Greater => Some(Side::Buy),
            std::cmp::Ordering::Less => Some(Side::Sell),
            std::cmp::Ordering::Equal => None,
        }
    }

    fn on_mark(&mut self, price: Decimal, view: &View<'_>) -> Vec<Intent> {
        // Not a price: kept as the fresh mark, it would push the book out of a close's price.
        if price <= Decimal::ZERO {
            return Vec::new();
        }
        self.mark = Some((price, view.now_ms));
        self.meet_seam(view.now_ms);
        let mut intents = Vec::new();
        if let Some(closed) = self.candles.on_price(view.now_ms, price) {
            intents.extend(self.on_candle(&closed, view));
        }
        intents.extend(self.check_stop(price, view));
        intents
    }

    /// The first live mark after a warm-up: in the candle the loaded ones ended at, the live
    /// candles carry on from them; in any later one a candle has no price, and the RSI warms up
    /// from live data, as after any gap.
    fn meet_seam(&mut self, now_ms: u64) {
        let Some(seam) = self.seam.take() else { return };
        if self.config.interval.open_of(now_ms) == seam {
            return;
        }
        tracing::warn!(
            seam,
            now_ms,
            "rsi-momentum: the past candles miss the live ones, warming up again"
        );
        self.rsi.reset();
        self.last_rsi = None;
        self.history = Some(
            "Its past candles didn't meet the live prices (a candle went by at the start), so it \
             warms up from live data. "
                .into(),
        );
    }

    fn on_candle(&mut self, closed: &Closed, view: &View<'_>) -> Vec<Intent> {
        let cooldown = self.cooldown_left;
        self.cooldown_left = cooldown.saturating_sub(1);
        if closed.missed > 0 {
            tracing::warn!(
                missed = closed.missed,
                "rsi-momentum: candles with no price, the RSI warms up again"
            );
            self.rsi.reset();
            self.last_rsi = None;
            return Vec::new();
        }
        let prev = self.last_rsi;
        let rsi = self.rsi.push(closed.candle.close);
        self.last_rsi = rsi;
        tracing::debug!(close = %closed.candle.close, ?rsi, ?prev, "rsi-momentum: candle closed");
        match decide::on_close(&self.config.params(), prev, rsi, self.held_side(view), cooldown) {
            Signal::Enter(side) => self.enter(side, view),
            Signal::Exit => self.start_close(view),
            Signal::Hold => Vec::new(),
        }
    }

    /// Close the position once the price is past the stop. Runs on every mark, not only at a
    /// candle close.
    fn check_stop(&mut self, price: Decimal, view: &View<'_>) -> Vec<Intent> {
        if self.closing.is_some() {
            return Vec::new();
        }
        let Some(side) = self.held_side(view) else { return Vec::new() };
        let entry = self.avg_entry(view).unwrap_or(Decimal::ZERO);
        if !decide::stop_hit(side, entry, price, self.config.stop_loss_pct) {
            return Vec::new();
        }
        tracing::warn!(%side, %entry, %price, pct = %self.config.stop_loss_pct, "rsi-momentum: stop loss");
        self.start_close(view)
    }

    /// Why an entry cannot go in now, if it cannot.
    fn entry_blocked(&self, view: &View<'_>) -> Option<&'static str> {
        if self.closing.is_some() {
            Some("a close is under way")
        } else if self.entry_sent_ms.is_some() {
            Some("an entry is in flight")
        } else if !self.position(view).is_zero() {
            Some("not flat")
        } else if self.venue != Some(Decimal::ZERO) {
            // A missed Trade, a manual trade, or a venue the tick could not read: not flat.
            Some("the venue does not read flat")
        } else if self.fresh_mark(view.now_ms).is_none() {
            Some("the mark is stale")
        } else {
            None
        }
    }

    fn enter(&mut self, side: Side, view: &View<'_>) -> Vec<Intent> {
        if let Some(why) = self.entry_blocked(view) {
            tracing::info!(%side, why, "rsi-momentum: entry signal skipped");
            return Vec::new();
        }
        let step = self.market(view).and_then(|m| m.facts.step_size).filter(|s| *s > Decimal::ZERO);
        let (Some(mark), Some(step)) = (self.fresh_mark(view.now_ms), step) else {
            tracing::warn!("rsi-momentum: no fresh mark or no size step, no entry");
            return Vec::new();
        };
        let c = &self.config;
        let qty = decide::entry_qty(c.notional_usd, mark, step, c.max_position, Decimal::ZERO);
        if qty <= Decimal::ZERO {
            tracing::warn!(%mark, %step, "rsi-momentum: entry below one step, skipped");
            return Vec::new();
        }
        let Some(book) = self.market(view).and_then(MarketView::book) else {
            tracing::warn!("rsi-momentum: no book, no entry");
            return Vec::new();
        };
        let Some(mid) = two_sided_mid(book).filter(|m| within_bps(*m, mark, MAX_IOC_BPS_FROM_MID))
        else {
            tracing::warn!(%mark, "rsi-momentum: no live, sane two-sided book near the mark, no entry");
            return Vec::new();
        };
        let touch = touch(book, side).unwrap_or(mid);
        let Some(price) =
            capped_ioc_price(side, touch, mid, self.config.slippage_bps, MAX_IOC_BPS_FROM_MID)
        else {
            return Vec::new();
        };
        self.entry_sent_ms = Some(view.now_ms);
        vec![Intent::on(
            BULLET,
            ExchangeIntent::Place(NewOrder {
                // The venue holds the stop from the first fill (the host keeps it at the entry price
                // after).
                protect: decide::stop_price(side, mark, self.config.stop_loss_pct)
                    .map(Protection::stop_at),
                symbol: self.config.symbol.clone(),
                side,
                order_type: OrderType::Market,
                price,
                quantity: qty,
                client_id: Some(self.client_ids.issue()),
                reduce_only: false,
            }),
        )]
    }

    fn start_close(&mut self, view: &View<'_>) -> Vec<Intent> {
        self.cooldown_left = self.config.cooldown_candles;
        if self.closing.is_none() {
            tracing::info!("rsi-momentum: closing");
            self.closing = Some(Closing { attempts: 0 });
        }
        self.close_attempt(view)
    }

    /// One reduce-only close of the whole position, crossing wider each attempt. Called again
    /// every tick until the position reads flat.
    fn close_attempt(&mut self, view: &View<'_>) -> Vec<Intent> {
        let Some(mut closing) = self.closing else { return Vec::new() };
        if self.held(view).is_zero() {
            tracing::info!("rsi-momentum: flat");
            self.closing = None;
            return Vec::new();
        }
        closing.attempts = closing.attempts.saturating_add(1);
        self.closing = Some(closing);
        let mult = Decimal::from(CLOSE_SLIPPAGE_MULT * closing.attempts.min(CLOSE_ATTEMPTS));
        let mut intents: Vec<Intent> = self.close_order(view, mult).into_iter().collect();
        // Past its attempts the close keeps going every tick, and the owner is asked once. The
        // alert stays raised until the next start (the owner's resume) resolves it.
        if closing.attempts == CLOSE_ATTEMPTS {
            let text = format!(
                "rsi-momentum {}: a close keeps failing and the position is still open; the bot \
                 retries every tick. Check the venue.",
                self.config.symbol
            );
            intents.push(Intent::Notify(Alert::Raise { key: ALERT_CLOSE.into(), text }));
        }
        intents
    }

    /// A reduce-only IOC for the whole position, crossing `slippage_bps x mult` and capped around
    /// the fresh book's mid when it agrees with the mark, else the fresh mark, else the entry
    /// price.
    fn close_order(&mut self, view: &View<'_>, mult: Decimal) -> Option<Intent> {
        let held = self.held(view);
        let side = if held > Decimal::ZERO { Side::Sell } else { Side::Buy };
        let stale_ms = self.config.stale_secs.saturating_mul(1_000);
        let fresh = self.fresh_mark(view.now_ms);
        let age = self.book_ms.map(|at| view.now_ms.saturating_sub(at));
        let book = self.market(view).and_then(MarketView::book);
        let (reference, base) = if let Some(rb) =
            close_reference(book, age, side, fresh, stale_ms, MAX_IOC_BPS_FROM_MID)
        {
            rb
        } else if let Some(m) = self.avg_entry(view) {
            (m, m)
        } else {
            tracing::error!("rsi-momentum: no book, mark or entry price to cap a close on");
            return None;
        };
        let slip = self.config.slippage_bps.checked_mul(mult)?;
        let price = capped_ioc_price(side, base, reference, slip, MAX_IOC_BPS_FROM_MID)?;
        Some(Intent::on(
            BULLET,
            ExchangeIntent::Place(NewOrder {
                protect: None,
                symbol: self.config.symbol.clone(),
                side,
                order_type: OrderType::Market,
                price,
                quantity: held.abs(),
                client_id: Some(self.client_ids.issue()),
                reduce_only: true,
            }),
        ))
    }

    fn on_tick(&mut self, view: &View<'_>) -> Vec<Intent> {
        let settle = ENTRY_SETTLE_SECS.saturating_mul(1_000);
        if self.entry_sent_ms.is_some_and(|t| view.now_ms.saturating_sub(t) >= settle) {
            self.entry_sent_ms = None;
        }
        let mut intents = self.close_attempt(view);
        if let Some(mark) = self.fresh_mark(view.now_ms) {
            intents.extend(self.check_stop(mark, view));
        }
        intents
    }
}

impl StrategyAdapter for RsiMomentum {
    type Report = RsiReport;

    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        let Some(market) = self.market(view) else {
            return Err(StrategyError(format!(
                "The host shows no {} market on {BULLET}: rsi-momentum trades only that one.",
                self.config.symbol
            )));
        };
        if market.facts.step_size.is_none() {
            tracing::warn!(symbol = %self.config.symbol, "rsi-momentum: the venue gives no size step yet");
        }
        tracing::info!(
            symbol = %self.config.symbol,
            interval = self.config.interval.name(),
            rsi_period = self.config.rsi_period,
            "rsi-momentum: ready, warming up"
        );
        self.book_ms = market.book().map(|_| view.now_ms);
        if let Some(read) = self.venue_read(view) {
            self.venue = Some(read);
        }
        // What an earlier run left raised for the owner: this start (their resume) acknowledges it.
        Ok(vec![Intent::Notify(Alert::Resolve { key: ALERT_CLOSE.into() })])
    }

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent> {
        self.observe(event, view);
        match (self.ours(event), event) {
            (Some(MarketEvent::Mark(price)), _) => self.on_mark(*price, view),
            (Some(MarketEvent::Fill(_)), _) => {
                self.entry_sent_ms = None;
                Vec::new()
            }
            (_, Event::Tick) => self.on_tick(view),
            (
                _,
                Event::Outcome {
                    intent: Intent::On { intent: ExchangeIntent::Place(order), .. },
                    result,
                },
            ) if !result.success && !order.reduce_only => {
                tracing::warn!(error = ?result.error, "rsi-momentum: entry not taken");
                self.entry_sent_ms = None;
                Vec::new()
            }
            // An entry that may have landed (`Unknown`) or was not carried out (`Failed`) keeps its
            // slot until its fill or `ENTRY_SETTLE_SECS`: never a second entry over it.
            // It asks for no command, write or close of the host's, so none is ever answered.
            _ => Vec::new(),
        }
    }

    /// Flatten: a reduce-only close of what the venue holds (the host reads it just before); what
    /// it leaves, the host's own close takes (`StrategyHost::with_flatten`).
    fn stop(&mut self, view: &View<'_>) -> Vec<Intent> {
        tracing::info!("rsi-momentum: winding down, flattening");
        self.closing = None;
        if let Some(read) = self.venue_read(view) {
            self.venue = Some(read);
        }
        if self.held(view).is_zero() {
            return Vec::new();
        }
        self.close_order(view, Decimal::from(CLOSE_SLIPPAGE_MULT)).into_iter().collect()
    }

    /// The stop loss, from the entry: the venue holds it while the bot is down.
    fn protective_stop(&self, view: &View<'_>) -> Option<Protection> {
        let side = self.held_side(view)?;
        decide::stop_price(side, self.avg_entry(view)?, self.config.stop_loss_pct)
            .map(Protection::stop_at)
    }

    /// A close under way (the stop or the exit band fired) is finished at a kept shutdown too.
    fn may_keep(&self) -> bool {
        self.closing.is_none()
    }

    fn report(&self) -> RsiReport {
        RsiReport {
            symbol: self.config.symbol.clone(),
            interval: self.config.interval.name().to_string(),
            rsi: self.last_rsi.map(|v| v.round_dp(2)),
            closes_to_warm: self.rsi.closes_needed(),
            mark: self.mark.map(|(m, _)| m.round_dp(2)),
            cooldown_candles_left: self.cooldown_left,
            close_attempts: self.closing.map(|c| c.attempts),
            close_failing: self.closing.is_some_and(|c| c.attempts >= CLOSE_ATTEMPTS),
            history: self.history.clone(),
        }
    }

    /// The RSI's warm-up: `rsi_period + 1` closes of the strategy's interval, and up to ten
    /// times that so the seed is smoothed away.
    fn history(&self) -> Option<HistoryNeed> {
        let need = self.config.rsi_period.saturating_add(1);
        let want = need.saturating_mul(10).min(MAX_HISTORY_CANDLES).max(need);
        Some(HistoryNeed { interval: self.config.interval, want, need })
    }

    /// The loaded closes seed the RSI as if the bot had run through them; the first live mark
    /// must then come in the candle they end at ([`Self::meet_seam`]).
    fn warm_up(&mut self, warmup: Warmup) {
        let interval = self.config.interval.name();
        match warmup {
            Warmup::Loaded { candles, source, now_ms } => {
                self.rsi.reset();
                for candle in &candles {
                    self.last_rsi = self.rsi.push(candle.close);
                }
                self.seam = Some(self.config.interval.open_of(now_ms));
                self.history = Some(format!(
                    "It warmed up on {} past {interval} candles from {source}. ",
                    candles.len()
                ));
            }
            Warmup::Unavailable(why) => {
                self.history = Some(format!(
                    "It couldn't load past prices ({why}), so it warms up from live data. "
                ));
            }
        }
    }
}

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod tests;
