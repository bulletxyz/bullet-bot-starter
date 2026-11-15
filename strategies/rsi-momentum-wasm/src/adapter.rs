//! RSI momentum as a [`StrategyAdapter`]: bullet-bots' `rsi-momentum` (`adapter.rs`) on the SDK.
//! Candles from the mark price, the RSI at each close, and the entries, exits and stop it asks for
//! as intents. The bot owns everything around them: the position, the order gate's limits, paper,
//! the daily loss stop and the flatten on wind-down.
//!
//! Rules kept from the built-in (its tests are in `adapter_tests.rs`; the parity fixtures replay the
//! built-in's own answers):
//! - no trading during warm-up (`rsi_period + 1` closes) or after a gap in the data (the RSI warms
//!   up again); an entry needs a cross, a fresh mark, a live two-sided book near it, and is sized
//!   to `notional_usd` within `max_position` when given;
//! - every IOC is priced within [`MAX_IOC_BPS_FROM_MID`] of the book's mid (a close without a live
//!   book: of the mark, else the entry price);
//! - a close is reduce-only, sized to the venue's position, retried every tick with a wider cross;
//!   after [`CLOSE_ATTEMPTS`] the report says `close_failing` and the closes go on;
//! - no entry while a close is under way, an entry is in flight, or a position is held.
//!
//! What the built-in does that the `bb:strategy@0.4.0` world cannot carry:
//! - the warm-up on past candles (`history`, `warm_up`): no candle read in the world, so it always
//!   warms up from live marks, as the built-in does when its load fails;
//! - the `rsi_momentum_close_failing` alert (`Intent::Notify`): the world has no alerts; the
//!   report's `close_failing` flag and headline say it instead;
//! - its log lines (`tracing`): a component has nowhere to log.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal prices, sizes and bps far below 7.9e28; attempt counters bounded by CLOSE_ATTEMPTS"
)]

use bb_strategy_sdk::{
    Decimal, Event, Intent, NewOrder, OrderType, Side, StrategyAdapter, StrategyError, View,
    signed_position,
};
use bb_strategy_support::ioc::{capped_ioc_price, two_sided_mid, within_bps};
use bb_strategy_support::{ClientIdIssuer, close_reference, touch};

use crate::candles::{CandleBuilder, Closed};
use crate::config::RsiMomentumSettings;
use crate::decide::{self, Intent as Signal};
use crate::limits::{CLOSE_ATTEMPTS, CLOSE_SLIPPAGE_MULT, ENTRY_SETTLE_SECS, MAX_IOC_BPS_FROM_MID};
use crate::report::RsiReport;
use crate::rsi::Rsi;

/// What the report says about the warm-up: a plugin gets no past candles.
pub const LIVE_WARMUP: &str = "A plugin gets no past candles, so it warms up from live data. ";

/// A close under way: how many attempts it has made.
#[derive(Debug, Clone, Copy)]
struct Closing {
    attempts: u32,
}

#[derive(Debug)]
pub struct RsiMomentum {
    config: RsiMomentumSettings,
    /// The market traded: the view's, from the start.
    symbol: String,
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
    history: Option<String>,
}

impl RsiMomentum {
    #[must_use]
    pub fn new(config: RsiMomentumSettings) -> Self {
        Self {
            candles: CandleBuilder::new(config.interval),
            rsi: Rsi::new(config.rsi_period),
            symbol: config.symbol.clone().unwrap_or_default(),
            config,
            last_rsi: None,
            mark: None,
            cooldown_left: 0,
            closing: None,
            entry_sent_ms: None,
            venue: None,
            book_ms: None,
            client_ids: ClientIdIssuer::default(),
            history: None,
        }
    }

    fn fresh_mark(&self, now_ms: u64) -> Option<Decimal> {
        let limit = self.config.stale_secs.saturating_mul(1_000);
        self.mark.filter(|(_, at)| now_ms.saturating_sub(*at) <= limit).map(|(m, _)| m)
    }

    /// The venue's position as the host last read it (`View::positions`: the start's and a tick's
    /// read), kept until a fill or a tick that did not read it; and when the last book came.
    fn observe(&mut self, event: &Event, view: &View<'_>) {
        match (event, view.positions) {
            (_, Some(positions)) => self.venue = Some(signed_position(positions, view.symbol)),
            (Event::Fill(_) | Event::Tick, None) => self.venue = None,
            _ => {}
        }
        if matches!(event, Event::Book) {
            self.book_ms = Some(view.now_ms);
        }
    }

    /// The position a close is sized and ended on: the venue's as the host last read it, else the
    /// host's.
    fn held(&self, view: &View<'_>) -> Decimal {
        self.venue.unwrap_or(view.position)
    }

    fn held_side(view: &View<'_>) -> Option<Side> {
        match view.position.cmp(&Decimal::ZERO) {
            std::cmp::Ordering::Greater => Some(Side::Buy),
            std::cmp::Ordering::Less => Some(Side::Sell),
            std::cmp::Ordering::Equal => None,
        }
    }

    fn on_mark(&mut self, price: Decimal, view: &View<'_>) -> Vec<Intent> {
        self.mark = Some((price, view.now_ms));
        let mut intents = Vec::new();
        if let Some(closed) = self.candles.on_price(view.now_ms, price) {
            intents.extend(self.on_candle(&closed, view));
        }
        intents.extend(self.check_stop(price, view));
        intents
    }

    fn on_candle(&mut self, closed: &Closed, view: &View<'_>) -> Vec<Intent> {
        let cooldown = self.cooldown_left;
        self.cooldown_left = cooldown.saturating_sub(1);
        if closed.missed > 0 {
            // Candles with no price: the RSI warms up again.
            self.rsi.reset();
            self.last_rsi = None;
            return Vec::new();
        }
        let prev = self.last_rsi;
        let rsi = self.rsi.push(closed.candle.close);
        self.last_rsi = rsi;
        match decide::on_close(&self.config.params(), prev, rsi, Self::held_side(view), cooldown) {
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
        let Some(side) = Self::held_side(view) else { return Vec::new() };
        let entry = view.avg_entry.unwrap_or(Decimal::ZERO);
        if !decide::stop_hit(side, entry, price, self.config.stop_loss_pct) {
            return Vec::new();
        }
        self.start_close(view)
    }

    /// Why an entry cannot go in now, if it cannot.
    fn entry_blocked(&self, view: &View<'_>) -> Option<&'static str> {
        if self.closing.is_some() {
            Some("a close is under way")
        } else if self.entry_sent_ms.is_some() {
            Some("an entry is in flight")
        } else if !view.position.is_zero() {
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
        if self.entry_blocked(view).is_some() {
            return Vec::new();
        }
        let step = view.market.step_size.filter(|s| *s > Decimal::ZERO);
        let (Some(mark), Some(step)) = (self.fresh_mark(view.now_ms), step) else {
            return Vec::new();
        };
        let c = &self.config;
        let qty = decide::entry_qty(c.notional_usd, mark, step, c.position_cap(), Decimal::ZERO);
        if qty <= Decimal::ZERO {
            return Vec::new();
        }
        let Some(book) = view.book else { return Vec::new() };
        let Some(mid) = two_sided_mid(book).filter(|m| within_bps(*m, mark, MAX_IOC_BPS_FROM_MID))
        else {
            return Vec::new();
        };
        let touch = touch(book, side).unwrap_or(mid);
        let Some(price) =
            capped_ioc_price(side, touch, mid, self.config.slippage_bps, MAX_IOC_BPS_FROM_MID)
        else {
            return Vec::new();
        };
        self.entry_sent_ms = Some(view.now_ms);
        vec![Intent::Place(NewOrder {
            symbol: view.symbol.to_string(),
            side,
            order_type: OrderType::Market,
            price,
            quantity: qty,
            client_id: Some(self.client_ids.issue()),
            reduce_only: false,
            protect: None,
        })]
    }

    fn start_close(&mut self, view: &View<'_>) -> Vec<Intent> {
        self.cooldown_left = self.config.cooldown_candles;
        if self.closing.is_none() {
            self.closing = Some(Closing { attempts: 0 });
        }
        self.close_attempt(view)
    }

    /// One reduce-only close of the whole position, crossing wider each attempt. Called again
    /// every tick until the position reads flat.
    fn close_attempt(&mut self, view: &View<'_>) -> Vec<Intent> {
        let Some(mut closing) = self.closing else { return Vec::new() };
        if self.held(view).is_zero() {
            self.closing = None;
            return Vec::new();
        }
        closing.attempts = closing.attempts.saturating_add(1);
        self.closing = Some(closing);
        let mult = Decimal::from(CLOSE_SLIPPAGE_MULT * closing.attempts.min(CLOSE_ATTEMPTS));
        // Past its attempts the close keeps going every tick. The built-in raises an alert then
        // (`Intent::Notify`); the world has none, so the report's `close_failing` says it.
        self.close_order(view, mult).into_iter().collect()
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
        // None without a book, mark or entry price to cap a close on.
        let (reference, base) =
            close_reference(view.book, age, side, fresh, stale_ms, MAX_IOC_BPS_FROM_MID)
                .or_else(|| view.avg_entry.map(|m| (m, m)))?;
        let slip = self.config.slippage_bps.checked_mul(mult)?;
        let price = capped_ioc_price(side, base, reference, slip, MAX_IOC_BPS_FROM_MID)?;
        Some(Intent::Place(NewOrder {
            symbol: view.symbol.to_string(),
            side,
            order_type: OrderType::Market,
            price,
            quantity: held.abs(),
            client_id: Some(self.client_ids.issue()),
            reduce_only: true,
            protect: None,
        }))
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
        if let Some(symbol) = self.config.symbol.as_deref()
            && symbol != view.symbol
        {
            return Err(StrategyError(format!(
                "rsi-momentum: the settings name {symbol}, the bot trades {}",
                view.symbol
            )));
        }
        view.symbol.clone_into(&mut self.symbol);
        // The bot's own ids, seeded from its clock (a component has no other).
        self.client_ids = ClientIdIssuer::own(view.now_ms);
        self.history = Some(LIVE_WARMUP.into());
        self.book_ms = view.book.map(|_| view.now_ms);
        if let Some(positions) = view.positions {
            self.venue = Some(signed_position(positions, view.symbol));
        }
        // The built-in resolves its close-failing alert here (`Intent::Notify`): none to resolve.
        Ok(Vec::new())
    }

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent> {
        self.observe(event, view);
        match event {
            Event::Mark(price) => self.on_mark(*price, view),
            Event::Fill(_) => {
                self.entry_sent_ms = None;
                Vec::new()
            }
            Event::Tick => self.on_tick(view),
            Event::Outcome { intent: Intent::Place(order), result }
                if !result.success && !order.reduce_only =>
            {
                // The entry was not taken.
                self.entry_sent_ms = None;
                Vec::new()
            }
            // An entry that may have landed (`Unknown`) keeps its slot until its fill or
            // `ENTRY_SETTLE_SECS`: never a second entry over it.
            _ => Vec::new(),
        }
    }

    /// Flatten: a reduce-only close of what the venue holds (the host reads it just before); what
    /// it leaves, the host's own close takes.
    fn stop(&mut self, view: &View<'_>) -> Vec<Intent> {
        self.closing = None;
        if let Some(positions) = view.positions {
            self.venue = Some(signed_position(positions, view.symbol));
        }
        if self.held(view).is_zero() {
            return Vec::new();
        }
        self.close_order(view, Decimal::from(CLOSE_SLIPPAGE_MULT)).into_iter().collect()
    }

    fn report(&self) -> RsiReport {
        RsiReport {
            symbol: self.symbol.clone(),
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
}

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod tests;
