//! Avellaneda-Stoikov as a [`StrategyAdapter`]: bullet-bots' `avellaneda-stoikov` actor on the SDK.
//! A ladder of quotes around a reservation price that inventory skews, requoted on every book
//! (or reference) update, kept in place unless a level drifted past its amend threshold, and
//! reconciled against the venue's open orders. The bot owns everything around it: the position, the
//! order gate's limits, paper and the daily loss stop.
//!
//! Rules kept from the built-in (its recorded calls are the parity fixtures):
//! - the quotes: `quote_ladder` over sigma from the rolling mid samples; a side is off once the
//!   position reaches `max_position` on it; level `i` is `order_size × (1 + i × step)`;
//! - a requote cancels the levels no longer wanted, amends the drifted ones and places the missing
//!   ones, in that order; a `PostOnly` rung that would cross the book is skipped (and its resting
//!   order cancelled when it crosses too);
//! - at most one requote per `min_refresh_interval_ms`, a forced one every `order_refresh_secs` on a
//!   tick, and one after every fill; fair-value mode stops requoting when the reference is stale.
//!
//! Where the world differs from the built-in's actor (which called the broker and read the OS clock):
//! - time is `View::now_ms`; a call's result arrives as `Event::Outcome` after the intents, so the
//!   quote slots are updated when the intents are sent and corrected on the outcome: a refused
//!   place drops its slot, a refused cancel puts the slot back, a refused amend restores the old
//!   slot (as the actor did from the results it got synchronously);
//! - the reconcile with the venue's open orders is an `Intent::Request` read answered later by
//!   `Event::Answer`; its orphan cancels (at most 64) go out then, not in the requote's call;
//!   the actor's reconcile signal on a stream reconnect is `View::reconnected`;
//! - a call that errored after it may have reached the venue (`Event::Unknown`) is held, never
//!   retried: the slot stays and the next reconcile settles it (the actor dropped the whole
//!   requote on a broker error and tried again);
//! - the reference price is a book on the `reference` leg (`Event::Leg`), its age the bot's clock;
//! - not carried: the actor's shutdown request on a permanently disconnected broker (the bot does
//!   that itself), its log lines, and the "no `fees` block" warning.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal prices, sizes and bps far below 7.9e28; constant divisors; slot counts where \
              the subtrahend is a filtered subset"
)]

use std::collections::{BTreeMap, HashSet};

use bb_strategy_sdk::{
    AmendOrder, Answer, CancelOrder, Decimal, Event, Intent, NewOrder, Order, OrderBook,
    OrderStatus, OrderType, ReadQuery, Side, StrategyAdapter, StrategyError, TRADE_LEG, VenueCall,
    View,
};
use bb_strategy_support::{BookExt, ClientIdIssuer};

use crate::REFERENCE_LEG;
use crate::config::AvellanedaStoikovSettings;
use crate::model::{LadderRung, Quote, SpreadBounds, quote_ladder};
use crate::report::AsReport;
use crate::volatility::Volatility;

/// A bot takes at most this many intents from a plugin in one call.
const MAX_INTENTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
struct QuoteSlot {
    side: Side,
    level: usize,
    price: Decimal,
    size: Decimal,
    client_id: Option<String>,
    order_id: Option<String>,
}

/// What `reconcile_orders` decided, before anything is sent.
struct Reconciled {
    kept: Vec<QuoteSlot>,
    amends: Vec<(usize, AmendOrder, QuoteSlot)>,
    places: Vec<(NewOrder, QuoteSlot)>,
    cancels: Vec<(CancelOrder, QuoteSlot)>,
}

#[derive(Debug)]
pub struct AvellanedaStoikov {
    config: AvellanedaStoikovSettings,
    symbol: String,
    slots: Vec<QuoteSlot>,
    /// Slots whose cancel was sent, until its outcome: a refused cancel puts the slot back.
    cancelling: Vec<QuoteSlot>,
    /// Amends sent, by the new client id, with the slot they replaced: a refused amend restores it.
    amending: BTreeMap<String, QuoteSlot>,
    net_position: Decimal,
    total_fills: u64,
    client_ids: ClientIdIssuer,
    volatility: Volatility,
    book: Option<OrderBook>,
    last_mid: Option<Decimal>,
    last_quote_ms: Option<u64>,
    reference_seen_ms: Option<u64>,
    last_reconcile_ms: Option<u64>,
    /// A reconcile is wanted at the next requote that sends something (a reconnect, an unknown).
    reconcile_wanted: bool,
    /// The id of the open-orders read in flight.
    reading: Option<u64>,
    next_request: u64,
    /// The last ladder's inner quote, for the report.
    last_inner: Option<Quote>,
    /// The bot clock at the last call, for the report.
    last_now_ms: u64,
}

impl AvellanedaStoikov {
    #[must_use]
    pub fn new(config: AvellanedaStoikovSettings) -> Self {
        let volatility = Volatility::new(config.vol_window_secs);
        Self {
            config,
            symbol: String::new(),
            slots: Vec::new(),
            cancelling: Vec::new(),
            amending: BTreeMap::new(),
            net_position: Decimal::ZERO,
            total_fills: 0,
            client_ids: ClientIdIssuer::new(),
            volatility,
            book: None,
            last_mid: None,
            last_quote_ms: None,
            reference_seen_ms: None,
            last_reconcile_ms: None,
            reconcile_wanted: false,
            reading: None,
            next_request: 1,
            last_inner: None,
            last_now_ms: 0,
        }
    }

    fn reference_is_stale(&self, now_ms: u64) -> bool {
        match self.reference_seen_ms {
            None => true,
            Some(t) => {
                now_ms.saturating_sub(t)
                    > self.config.reference_stale_secs.max(1).saturating_mul(1000)
            }
        }
    }

    fn build_ladder(&self, mid: Decimal) -> Option<(Quote, Vec<LadderRung>)> {
        use rust_decimal::prelude::ToPrimitive;
        let sigma = self.volatility.sigma()?;
        let mid_f = mid.to_f64()?;
        let gamma = self.config.gamma.to_f64()?;
        let kappa = self.config.kappa.to_f64()?;
        #[expect(clippy::cast_precision_loss, reason = "a horizon in seconds, far below 2^53")]
        let tau = self.config.order_horizon_secs as f64;
        let max_pos = self.config.max_position.to_f64()?;
        let target = self.config.inventory_target.to_f64().unwrap_or(0.0);
        let pos = self.net_position.to_f64()?;
        let inv_norm = if max_pos > 0.0 { (pos - target) / max_pos } else { 0.0 };
        let level_step_bps = self.config.order_level_spread_bps.to_f64()?;
        let levels = self.config.order_levels.max(1);
        Some(quote_ladder(
            mid_f,
            inv_norm,
            gamma,
            kappa,
            sigma,
            tau,
            SpreadBounds {
                min_half_spread_bps: self.config.min_half_spread_bps.to_f64()?,
                max_half_spread_bps: self.config.max_half_spread_bps.to_f64()?,
            },
            levels,
            level_step_bps,
        ))
    }

    fn should_force_refresh(&self, now_ms: u64) -> bool {
        match self.last_quote_ms {
            None => true,
            Some(last) => {
                now_ms.saturating_sub(last)
                    >= self.config.order_refresh_secs.max(1).saturating_mul(1000)
            }
        }
    }

    fn size_for_level(&self, level: usize) -> Decimal {
        let step = self.config.order_level_amount_step;
        if step.is_zero() {
            return self.config.order_size;
        }
        self.config.order_size * (Decimal::ONE + step * Decimal::from(level as u64))
    }

    fn cancel_of(&self, slot: &QuoteSlot) -> CancelOrder {
        CancelOrder {
            symbol: self.symbol.clone(),
            order_id: slot.order_id.clone().unwrap_or_default(),
            client_id: slot.client_id.clone(),
        }
    }

    fn quote_order(&self, slot: &QuoteSlot, order_type: OrderType, client_id: &str) -> NewOrder {
        NewOrder {
            symbol: self.symbol.clone(),
            side: slot.side,
            order_type,
            price: slot.price,
            quantity: slot.size,
            client_id: Some(client_id.to_owned()),
            reduce_only: false,
            protect: None,
        }
    }

    /// Diff the wanted `(side, level, price, size)` rungs against the slots: which to keep, amend,
    /// place and cancel. Nothing is sent or changed here but the ids issued.
    fn reconcile_orders(
        &mut self,
        wanted: &[(Side, usize, Decimal, Decimal)],
        mid: Decimal,
        order_type: OrderType,
        post_only: bool,
    ) -> Reconciled {
        let mut matched = vec![false; self.slots.len()];
        let mut out = Reconciled {
            kept: Vec::new(),
            amends: Vec::new(),
            places: Vec::new(),
            cancels: Vec::new(),
        };
        for (side, level, price, size) in wanted {
            let level_bps = self.config.amend_threshold_bps
                + self.config.amend_threshold_step_bps * Decimal::from(*level as u64);
            let threshold = mid * level_bps / Decimal::from(10_000);
            let find = |slots: &[QuoteSlot], matched: &[bool]| {
                slots
                    .iter()
                    .enumerate()
                    .find(|(i, s)| !matched[*i] && s.side == *side && s.level == *level)
                    .map(|(i, _)| i)
            };

            if post_only && self.book.as_ref().is_some_and(|b| b.would_cross(*side, *price)) {
                if let Some(idx) = find(&self.slots, &matched) {
                    let existing_also_crosses = self
                        .book
                        .as_ref()
                        .is_some_and(|b| b.would_cross(*side, self.slots[idx].price));
                    // A crossing resting order stays unmatched: the cancel sweep removes it.
                    if !existing_also_crosses {
                        matched[idx] = true;
                        out.kept.push(self.slots[idx].clone());
                    }
                }
                continue;
            }

            if let Some(idx) = find(&self.slots, &matched) {
                matched[idx] = true;
                let slot = self.slots[idx].clone();
                if (slot.price - *price).abs() < threshold
                    || (slot.order_id.is_none() && slot.client_id.is_none())
                {
                    out.kept.push(slot);
                    continue;
                }
                let new_cid = self.client_ids.issue();
                let new_slot = QuoteSlot {
                    side: *side,
                    level: *level,
                    price: *price,
                    size: *size,
                    client_id: Some(new_cid.clone()),
                    order_id: None,
                };
                let amend = AmendOrder {
                    cancel: self.cancel_of(&slot),
                    new_order: self.quote_order(&new_slot, order_type, &new_cid),
                };
                out.amends.push((idx, amend, new_slot));
            } else {
                let cid = self.client_ids.issue();
                let slot = QuoteSlot {
                    side: *side,
                    level: *level,
                    price: *price,
                    size: *size,
                    client_id: Some(cid.clone()),
                    order_id: None,
                };
                out.places.push((self.quote_order(&slot, order_type, &cid), slot));
            }
        }
        for (i, slot) in self.slots.iter().enumerate() {
            if matched[i] || (slot.order_id.is_none() && slot.client_id.is_none()) {
                continue;
            }
            out.cancels.push((self.cancel_of(slot), slot.clone()));
        }
        out
    }

    /// The requote: the intents that move the venue's quotes to the model's ladder, and the slots
    /// as they will be once the venue takes them.
    fn refresh_quotes(&mut self, now_ms: u64, mid: Decimal, reconnected: bool) -> Vec<Intent> {
        use rust_decimal::prelude::FromPrimitive;
        self.reconcile_wanted |= reconnected;
        let min_interval = self.config.min_refresh_interval_ms;
        if min_interval > 0
            && let Some(last) = self.last_quote_ms
            && now_ms.saturating_sub(last) < min_interval
        {
            return Vec::new();
        }
        if self.config.uses_reference() && self.reference_is_stale(now_ms) {
            return Vec::new();
        }
        let Some((inner, rungs)) = self.build_ladder(mid) else {
            return Vec::new();
        };
        self.last_inner = Some(inner);

        let can_buy = self.net_position < self.config.max_position;
        let can_sell = self.net_position > -self.config.max_position;
        let order_type = self.config.order_type;
        let mut wanted: Vec<(Side, usize, Decimal, Decimal)> = Vec::new();
        for rung in &rungs {
            let size = self.size_for_level(rung.level);
            if can_buy {
                let Some(bid) = Decimal::from_f64(rung.bid) else { return Vec::new() };
                wanted.push((Side::Buy, rung.level, bid, size));
            }
            if can_sell {
                let Some(ask) = Decimal::from_f64(rung.ask) else { return Vec::new() };
                wanted.push((Side::Sell, rung.level, ask, size));
            }
        }
        // A PostOnly order that would cross is refused by the venue, and a refused batch member
        // can sink the batch: pre-empt it. Limit and Market may cross.
        let post_only = matches!(order_type, OrderType::PostOnly);
        let plan = self.reconcile_orders(&wanted, mid, order_type, post_only);
        if plan.cancels.is_empty() && plan.amends.is_empty() && plan.places.is_empty() {
            // Nothing moved: `last_quote_ms` stays, so a tick's forced refresh still comes.
            return Vec::new();
        }

        let mut intents = Vec::new();
        let mut slots = plan.kept;
        for (cancel, slot) in plan.cancels {
            intents.push(Intent::Cancel(cancel));
            self.cancelling.push(slot);
        }
        let old = self.slots.clone();
        for (idx, amend, new_slot) in plan.amends {
            if let Some(cid) = new_slot.client_id.clone() {
                self.amending.insert(cid, old[idx].clone());
            }
            intents.push(Intent::Amend(amend));
            slots.push(new_slot);
        }
        for (order, slot) in plan.places {
            intents.push(Intent::Place(order));
            slots.push(slot);
        }
        self.slots = slots;
        self.last_quote_ms = Some(now_ms);

        let due = self.config.reconcile_interval_secs > 0
            && self.last_reconcile_ms.is_none_or(|t| {
                now_ms.saturating_sub(t) >= self.config.reconcile_interval_secs.saturating_mul(1000)
            });
        if (due || self.reconcile_wanted) && self.reading.is_none() {
            self.reconcile_wanted = false;
            let id = self.next_request;
            self.next_request = self.next_request.saturating_add(1);
            self.reading = Some(id);
            intents.push(Intent::Request {
                id,
                leg: TRADE_LEG.into(),
                call: VenueCall::Read(ReadQuery::OpenOrders { symbol: self.symbol.clone() }),
            });
        }
        intents
    }

    /// The venue's open orders against the slots: drop the phantoms (tracked, gone on the venue)
    /// and cancel the orphans (resting, untracked). A slot with no order id yet is held by its
    /// client id, so an in-flight place is not dropped.
    fn reconcile_with(&mut self, open: &[Order]) -> Vec<Intent> {
        let live_oids: HashSet<&str> = open.iter().map(|o| o.id.as_str()).collect();
        let live_clients: HashSet<&str> =
            open.iter().filter_map(|o| o.client_id.as_deref()).collect();
        self.slots.retain(|s| match s.order_id.as_deref() {
            Some(oid) => {
                live_oids.contains(oid)
                    || s.client_id.as_deref().is_some_and(|c| live_clients.contains(c))
            }
            None => true,
        });
        let tracked_oids: HashSet<&str> =
            self.slots.iter().filter_map(|s| s.order_id.as_deref()).collect();
        let tracked_clients: HashSet<&str> =
            self.slots.iter().filter_map(|s| s.client_id.as_deref()).collect();
        open.iter()
            .filter(|o| {
                let oid_tracked = tracked_oids.contains(o.id.as_str());
                let cid_tracked =
                    o.client_id.as_deref().is_some_and(|c| tracked_clients.contains(c));
                !oid_tracked && !cid_tracked
            })
            .take(MAX_INTENTS)
            .map(|o| {
                Intent::Cancel(CancelOrder {
                    symbol: self.symbol.clone(),
                    order_id: o.id.clone(),
                    client_id: o.client_id.clone(),
                })
            })
            .collect()
    }

    /// A slot the venue no longer has: by client id or order id, both only when known.
    fn drop_slots(&mut self, client_id: Option<&str>, order_id: &str) {
        let order_id = Some(order_id).filter(|s| !s.is_empty());
        self.slots.retain(|s| {
            let cid = client_id.is_some() && s.client_id.as_deref() == client_id;
            let oid = order_id.is_some() && s.order_id.as_deref() == order_id;
            !(cid || oid)
        });
    }

    fn on_outcome(&mut self, intent: &Intent, result: &bb_strategy_sdk::OrderResult) {
        match intent {
            Intent::Cancel(cancel) => {
                let at = self.cancelling.iter().position(|s| {
                    (cancel.client_id.is_some() && s.client_id == cancel.client_id)
                        || (!cancel.order_id.is_empty()
                            && s.order_id.as_deref() == Some(cancel.order_id.as_str()))
                });
                if let Some(at) = at {
                    let slot = self.cancelling.remove(at);
                    if !result.success {
                        // Retain tracking to retry.
                        self.slots.push(slot);
                    }
                }
            }
            Intent::Amend(amend) => {
                let Some(cid) = amend.new_order.client_id.as_deref() else { return };
                let Some(old) = self.amending.remove(cid) else { return };
                if result.success {
                    if let Some(slot) =
                        self.slots.iter_mut().find(|s| s.client_id.as_deref() == Some(cid))
                    {
                        slot.order_id.clone_from(&result.order_id);
                    }
                } else {
                    self.slots.retain(|s| s.client_id.as_deref() != Some(cid));
                    self.slots.push(old);
                }
            }
            Intent::Place(order) => {
                let Some(cid) = order.client_id.as_deref() else { return };
                if result.success {
                    if let Some(slot) =
                        self.slots.iter_mut().find(|s| s.client_id.as_deref() == Some(cid))
                    {
                        slot.order_id.clone_from(&result.order_id);
                    }
                } else {
                    self.slots.retain(|s| s.client_id.as_deref() != Some(cid));
                }
            }
            _ => {}
        }
    }

    /// The reference leg's book moved: its mid is the price now.
    fn on_reference(&mut self, symbol: &str, view: &View<'_>) -> Vec<Intent> {
        use rust_decimal::prelude::ToPrimitive;
        let Some(ref_symbol) = self.config.reference_symbol.as_deref() else {
            return Vec::new();
        };
        if !symbol.eq_ignore_ascii_case(ref_symbol) {
            return Vec::new();
        }
        // The leg's book under the symbol the event names, however the venue cases it.
        let Some(mid) = view
            .legs
            .iter()
            .find(|l| l.id == REFERENCE_LEG)
            .and_then(|l| {
                l.books.iter().find(|(k, _)| k.eq_ignore_ascii_case(symbol)).map(|(_, b)| b)
            })
            .and_then(OrderBook::midpoint)
        else {
            return Vec::new();
        };
        self.reference_seen_ms = Some(view.now_ms);
        self.last_mid = Some(mid);
        if let Some(f) = mid.to_f64() {
            self.volatility.push(f, view.now_ms);
        }
        self.refresh_quotes(view.now_ms, mid, false)
    }

    /// The venue's lifecycle update for one of its orders.
    fn on_order(&mut self, order: &Order) {
        if order.symbol != self.symbol {
            return;
        }
        let cid = order.client_id.as_deref();
        match order.status {
            // Terminal: gone from the venue's book, so stop tracking it.
            OrderStatus::Cancelled | OrderStatus::Rejected | OrderStatus::Filled => {
                self.drop_slots(cid, &order.id);
            }
            // A place ack: learn the venue's order id.
            OrderStatus::Open | OrderStatus::PartiallyFilled => {
                if let Some(cid) = cid
                    && !order.id.is_empty()
                    && let Some(slot) =
                        self.slots.iter_mut().find(|s| s.client_id.as_deref() == Some(cid))
                    && slot.order_id.is_none()
                {
                    slot.order_id = Some(order.id.clone());
                }
            }
        }
    }

    /// An intent whose call errored after it may have reached the venue: its slot stays as sent
    /// (a cancel's goes back), and the next requote reconciles with the venue.
    fn on_unknown(&mut self, intent: &Intent) {
        match intent {
            Intent::Cancel(cancel) => {
                let at = self.cancelling.iter().position(|s| {
                    (cancel.client_id.is_some() && s.client_id == cancel.client_id)
                        || (!cancel.order_id.is_empty()
                            && s.order_id.as_deref() == Some(cancel.order_id.as_str()))
                });
                if let Some(at) = at {
                    let slot = self.cancelling.remove(at);
                    self.slots.push(slot);
                }
            }
            Intent::Amend(amend) => {
                if let Some(cid) = amend.new_order.client_id.as_deref() {
                    self.amending.remove(cid);
                }
            }
            _ => return,
        }
        self.reconcile_wanted = true;
    }

    fn quote_of(&self, side: Side) -> Option<Decimal> {
        let prices = self.slots.iter().filter(|s| s.side == side).map(|s| s.price);
        if side == Side::Buy { prices.max() } else { prices.min() }
    }
}

impl StrategyAdapter for AvellanedaStoikov {
    type Report = AsReport;

    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        self.config.check_fee_floor()?;
        if let Some(symbol) = &self.config.symbol
            && symbol != view.symbol
        {
            return Err(StrategyError(format!(
                "avellaneda-stoikov: settings name {symbol}, the bot runs it on {}",
                view.symbol
            )));
        }
        view.symbol.clone_into(&mut self.symbol);
        self.net_position = view.position;
        self.client_ids = ClientIdIssuer::own(view.now_ms);
        // Seed the local book unconditionally; the mid and the volatility only in single-venue
        // mode (the reference feed owns them in fair-value mode).
        if !self.config.uses_reference() {
            let mid = view.book.and_then(OrderBook::midpoint).ok_or_else(|| {
                StrategyError(
                    "avellaneda-stoikov: No orderbook data available to compute mid price".into(),
                )
            })?;
            self.last_mid = Some(mid);
            if let Some(m) = rust_decimal::prelude::ToPrimitive::to_f64(&mid) {
                self.volatility.push(m, view.now_ms);
            }
        }
        self.book = view.book.cloned();
        Ok(vec![Intent::CancelAll])
    }

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent> {
        use rust_decimal::prelude::ToPrimitive;
        self.net_position = view.position;
        self.reconcile_wanted |= view.reconnected;
        let now = view.now_ms;
        self.last_now_ms = now;
        match event {
            Event::Book => {
                if !self.config.uses_reference()
                    && let Some(mid) = view.book.and_then(OrderBook::midpoint)
                {
                    self.last_mid = Some(mid);
                    if let Some(f) = mid.to_f64() {
                        self.volatility.push(f, now);
                    }
                }
                self.book = view.book.cloned();
                match self.last_mid {
                    Some(mid) => self.refresh_quotes(now, mid, false),
                    None => Vec::new(),
                }
            }
            Event::Leg { leg, symbol, event } if leg == REFERENCE_LEG => {
                if matches!(**event, Event::Book) {
                    self.on_reference(symbol, view)
                } else {
                    Vec::new()
                }
            }
            Event::Fill(trade) => {
                if trade.symbol != self.symbol {
                    return Vec::new();
                }
                self.total_fills = self.total_fills.saturating_add(1);
                self.drop_slots(trade.client_id.as_deref(), &trade.order_id);
                // Requote on the next tick so both sides follow the new reservation price.
                self.last_quote_ms = None;
                Vec::new()
            }
            Event::Order(order) => {
                self.on_order(order);
                Vec::new()
            }
            Event::Tick => {
                let Some(mid) = self.last_mid else { return Vec::new() };
                if self.should_force_refresh(now) {
                    return self.refresh_quotes(now, mid, view.reconnected);
                }
                Vec::new()
            }
            Event::Outcome { intent, result } => {
                self.on_outcome(intent, result);
                Vec::new()
            }
            Event::Unknown { intent, .. } => {
                self.on_unknown(intent);
                Vec::new()
            }
            Event::Answer { id, reply } => {
                if self.reading != Some(*id) {
                    return Vec::new();
                }
                self.reading = None;
                match reply {
                    Ok(Answer::OpenOrders(open)) => {
                        self.last_reconcile_ms = Some(now);
                        self.reconcile_with(open)
                    }
                    _ => Vec::new(),
                }
            }
            Event::Mark(_) | Event::Leg { .. } => Vec::new(),
        }
    }

    fn stop(&mut self, _view: &View<'_>) -> Vec<Intent> {
        // Market making never wants to pay a taker fee on the way out: cancel only, whatever the
        // reason.
        vec![Intent::CancelAll]
    }

    fn report(&self) -> AsReport {
        AsReport {
            symbol: self.symbol.clone(),
            mode: if self.config.uses_reference() { "fair-value" } else { "single-venue" },
            net_position: self.net_position,
            total_fills: self.total_fills,
            quotes: self.slots.len(),
            best_bid: self.quote_of(Side::Buy),
            best_ask: self.quote_of(Side::Sell),
            mid: self.last_mid,
            sigma: self.volatility.sigma(),
            vol_samples: self.volatility.sample_count(),
            reference_stale: self.config.uses_reference()
                && self.reference_seen_ms.is_some()
                && self.reference_is_stale(self.last_now_ms),
        }
    }
}

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod tests;
