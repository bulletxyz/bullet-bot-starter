//! The refresh (money safety): each side's wanted quote through the cap and the cross check, then
//! a place, a cancel, a reprice (one amend, or a cancel first near the cap), or nothing; and the
//! pull while stopped.

#![expect(clippy::arithmetic_side_effects, reason = "Decimal prices and bps far below 7.9e28")]

use bb_strategy_sdk::{
    AmendOrder, ExchangeIntent, Intent, NewOrder, OrderBook, OrderType, Side, View,
};
use rust_decimal::Decimal;

use super::{InFlight, MarketMaker, PULL_RETRY_MS, Slot, market_view, side_index};
use crate::cap::Caps;
use crate::pricing::Quote;

const BPS_SCALE: i64 = 10_000;

impl MarketMaker {
    pub(crate) fn on_book(&mut self, view: &View<'_>) -> Vec<Intent> {
        let caps = self.caps(view);
        let mv = market_view(&self.params, caps, self.tick, view, &self.settings.symbol);
        self.pricing.on_book(&mv);
        self.refresh(view)
    }

    fn due(&self, now_ms: u64) -> bool {
        self.forced
            || self.last_refresh_ms.is_none_or(|last| {
                now_ms.saturating_sub(last) >= self.params.refresh_secs.saturating_mul(1000)
            })
    }

    /// Quote, when due: nothing while stopped or without a two-sided book (what rests is pulled),
    /// nor while the last refresh's answers are not all in.
    pub(crate) fn refresh(&mut self, view: &View<'_>) -> Vec<Intent> {
        if self.stopped(view) || self.book(view).and_then(OrderBook::midpoint).is_none() {
            return self.pull_quotes(view, false);
        }
        if !self.in_flight.is_empty() || !self.due(view.now_ms) {
            return Vec::new();
        }
        self.last_refresh_ms = Some(view.now_ms);
        self.forced = false;
        let caps = self.caps(view);
        let mv = market_view(&self.params, caps, self.tick, view, &self.settings.symbol);
        let quotes = self.pricing.quote(&mv);
        // An order that may still rest beside a slot is cancelled first, and nothing is placed
        // until it is gone: two orders on a side could fill past the cap.
        let strays = std::mem::take(&mut self.strays);
        let place_nothing = !strays.is_empty();
        let mut intents: Vec<Intent> = Vec::new();
        for stray in strays {
            intents.push(Self::on_bullet(ExchangeIntent::Cancel(stray.cancel.clone())));
            self.in_flight.push(InFlight::Stray(stray));
        }
        for (side, wanted) in [(Side::Buy, quotes.bid), (Side::Sell, quotes.ask)] {
            intents.extend(self.quote_side(side, wanted, caps, view, place_nothing));
        }
        intents
    }

    /// One side of a refresh.
    fn quote_side(
        &mut self,
        side: Side,
        wanted: Option<Quote>,
        caps: Caps,
        view: &View<'_>,
        place_nothing: bool,
    ) -> Option<Intent> {
        let post_only = self.settings.order_type == OrderType::PostOnly;
        let (held, book) = (self.position(view), self.book(view));
        let quote = wanted
            .and_then(|q| {
                let qty = caps.allowed(side, q.size, q.price, held)?;
                Some(Quote { size: qty, ..q })
            })
            .filter(|q| !(post_only && book.is_some_and(|b| b.would_cross(side, q.price))));
        let slot = self.slots[side_index(side)].take();
        match (slot, quote) {
            (None, None) => None,
            (Some(slot), None) => Some(self.cancel(slot, None)),
            (None, Some(_)) if place_nothing => None,
            (None, Some(q)) => {
                let new = self.new_slot(side, q);
                Some(self.place(new))
            }
            (Some(slot), Some(q)) if !self.needs_refresh(&slot, q) => {
                self.slots[side_index(side)] = Some(slot);
                None
            }
            (Some(slot), Some(_)) if place_nothing => Some(self.cancel(slot, None)),
            (Some(slot), Some(q)) => {
                let new = self.new_slot(side, q);
                // A quote with no venue id may be a place still landing, which its cancel can
                // miss; near the cap, the old quote filling before the cancel lands would leave
                // the replacement past it. Either way the replacement waits for the cancel.
                let one_message = slot.order_id.is_some()
                    && !slot.unknown
                    && caps.passes_with_filled(side, q.size, q.price, held, slot.size);
                Some(if one_message { self.amend(slot, new) } else { self.cancel(slot, Some(new)) })
            }
        }
    }

    /// The resting quote should go for `wanted`: its size differs, or its price drifted past the
    /// threshold.
    fn needs_refresh(&self, slot: &Slot, wanted: Quote) -> bool {
        if slot.size != wanted.size {
            return true;
        }
        if wanted.price.is_zero() {
            return false;
        }
        let drift_bps = (slot.price - wanted.price).abs() / wanted.price * Decimal::from(BPS_SCALE);
        drift_bps >= self.params.refresh_threshold_bps
    }

    fn new_slot(&mut self, side: Side, q: Quote) -> Slot {
        Slot {
            side,
            price: q.price,
            size: q.size,
            client_id: self.client_ids.issue(),
            order_id: None,
            acked_ms: 0,
            unknown: false,
        }
    }

    fn order(&self, slot: &Slot) -> NewOrder {
        NewOrder {
            symbol: self.symbol().to_string(),
            side: slot.side,
            order_type: self.settings.order_type,
            price: slot.price,
            quantity: slot.size,
            client_id: Some(slot.client_id.clone()),
            reduce_only: false,
            protect: None,
        }
    }

    pub(crate) fn place(&mut self, slot: Slot) -> Intent {
        let intent = Self::on_bullet(ExchangeIntent::Place(self.order(&slot)));
        self.in_flight.push(InFlight::Place(slot));
        intent
    }

    fn amend(&mut self, old: Slot, new: Slot) -> Intent {
        let intent = Self::on_bullet(ExchangeIntent::Amend(AmendOrder {
            cancel: old.cancel(self.symbol()),
            new_order: self.order(&new),
        }));
        self.in_flight.push(InFlight::Amend { old, new });
        intent
    }

    fn cancel(&mut self, slot: Slot, then: Option<Slot>) -> Intent {
        let intent = Self::on_bullet(ExchangeIntent::Cancel(slot.cancel(self.symbol())));
        self.in_flight.push(InFlight::Cancel { slot, then });
        intent
    }

    /// A two-step reprice's cancel landed: its replacement, re-capped at the position now, unless
    /// a stop came or a stray appeared meanwhile.
    pub(crate) fn place_after_cancel(&mut self, mut new: Slot, view: &View<'_>) -> Vec<Intent> {
        if self.stopped(view) || !self.strays.is_empty() {
            return Vec::new();
        }
        let held = self.position(view);
        let Some(size) = self.caps(view).allowed(new.side, new.size, new.price, held) else {
            return Vec::new();
        };
        new.size = size;
        vec![self.place(new)]
    }

    /// Pull every quote while stopped: one `CancelOwn` while anything may rest (`force`: whatever
    /// is tracked), asked again at most every [`PULL_RETRY_MS`] while it fails.
    pub(crate) fn pull_quotes(&mut self, view: &View<'_>, force: bool) -> Vec<Intent> {
        let failed = self.pull.failed_ms;
        let tracked = self.slots.iter().any(Option::is_some) || !self.strays.is_empty();
        if !force && !tracked && failed.is_none() {
            self.forced = true;
            return Vec::new();
        }
        let waiting = failed.is_some_and(|t| view.now_ms.saturating_sub(t) < PULL_RETRY_MS);
        if self.pull.asked || (!force && waiting) {
            return Vec::new();
        }
        self.pull.asked = true;
        vec![self.cancel_own()]
    }
}
