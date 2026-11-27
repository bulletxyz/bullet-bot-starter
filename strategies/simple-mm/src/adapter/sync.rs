//! What became of each quote (money safety): the answers to a refresh, the order updates, and the
//! venue reads that settle a quote whose answer was lost. A quote is a slot only while the market
//! maker knows it rests (or may, its place unknown); one that may rest but is no slot is a stray.

use bb_strategy_sdk::{
    AmendOrder, CancelOrder, ExchangeIntent, Intent, Order, OrderResult, OrderStatus, View,
};

use super::{InFlight, MarketMaker, SETTLE_MS, Slot, Stray, side_index};

fn cancel_key(c: &CancelOrder) -> String {
    format!("{}/{}", c.order_id, c.client_id.as_deref().unwrap_or_default())
}

/// The key an intent and its answer share: the client id of what it places, or what it cancels.
fn key_of(intent: &Intent) -> Option<String> {
    let Intent::On { intent, .. } = intent else { return None };
    match intent {
        ExchangeIntent::Place(o) | ExchangeIntent::Amend(AmendOrder { new_order: o, .. }) => {
            o.client_id.clone()
        }
        ExchangeIntent::Cancel(c) => Some(cancel_key(c)),
        _ => None,
    }
}

impl InFlight {
    fn key(&self, symbol: &str) -> String {
        match self {
            Self::Place(new) | Self::Amend { new, .. } => new.client_id.clone(),
            Self::Cancel { slot, .. } => cancel_key(&slot.cancel(symbol)),
            Self::Stray(stray) => cancel_key(&stray.cancel),
        }
    }
}

/// `order` is the one `cancel` names, by venue id or client id.
fn names(cancel: &CancelOrder, order_id: &str, client_id: Option<&str>) -> bool {
    (!order_id.is_empty() && cancel.order_id == order_id)
        || client_id.is_some_and(|c| cancel.client_id.as_deref() == Some(c))
}

impl MarketMaker {
    fn take_in_flight(&mut self, intent: &Intent) -> Option<InFlight> {
        let key = key_of(intent)?;
        let at = self.in_flight.iter().position(|f| f.key(&self.settings.symbol) == key)?;
        Some(self.in_flight.remove(at))
    }

    fn set_slot(&mut self, slot: Slot) {
        let i = side_index(slot.side);
        self.slots[i] = Some(slot);
    }

    fn add_stray(&mut self, cancel: CancelOrder, now_ms: u64) {
        let key = cancel_key(&cancel);
        if !self.strays.iter().any(|s| cancel_key(&s.cancel) == key) {
            self.strays.push(Stray { cancel, since_ms: now_ms });
        }
    }

    pub(crate) fn on_outcome(
        &mut self,
        intent: &Intent,
        result: &OrderResult,
        view: &View<'_>,
    ) -> Vec<Intent> {
        let now_ms = view.now_ms;
        if matches!(intent, Intent::On { intent: ExchangeIntent::CancelOwn(_), .. }) {
            return self.pulled(result.success, view);
        }
        let Some(flight) = self.take_in_flight(intent) else {
            // A cancel no slot waits for: the old order of an amend whose cancel failed beside it.
            if let Intent::On { intent: ExchangeIntent::Cancel(c), .. } = intent
                && !result.success
            {
                self.add_stray(c.clone(), now_ms);
            }
            return Vec::new();
        };
        let acked = |mut slot: Slot| {
            slot.order_id.clone_from(&result.order_id);
            slot.acked_ms = now_ms;
            slot
        };
        match flight {
            InFlight::Place(new) => {
                if result.success {
                    self.set_slot(acked(new));
                }
            }
            InFlight::Amend { old, new } => {
                if result.success {
                    self.set_slot(acked(new));
                } else {
                    // Refused: its cancel may not have gone either, so the old quote may rest.
                    self.add_stray(old.cancel(&self.settings.symbol), now_ms);
                }
            }
            InFlight::Cancel { slot, then } => {
                if !result.success {
                    self.set_slot(slot);
                } else if let Some(new) = then {
                    return self.place_after_cancel(new, view);
                }
            }
            InFlight::Stray(stray) => {
                if !result.success {
                    self.strays.push(stray);
                }
            }
        }
        Vec::new()
    }

    /// The call that carried out `intent` failed as a whole: it was never sent, so what it asked
    /// is as it was.
    pub(crate) fn on_failed(&mut self, intent: &Intent, view: &View<'_>) -> Vec<Intent> {
        match intent {
            Intent::On { intent: ExchangeIntent::CancelOwn(_), .. } => {
                return self.pulled(false, view);
            }
            Intent::ClosePosition { id, .. } => return self.close_failed(*id),
            _ => {}
        }
        match self.take_in_flight(intent) {
            Some(InFlight::Amend { old: slot, .. } | InFlight::Cancel { slot, .. }) => {
                self.set_slot(slot);
            }
            Some(InFlight::Stray(stray)) => self.strays.push(stray),
            Some(InFlight::Place(_)) | None => {}
        }
        Vec::new()
    }

    /// The call errored after it may have reached the venue: a place (an amend's new order) is
    /// held as resting, its venue id unknown, so its side is not placed again until a read settles
    /// it; an amend's old quote may still rest too.
    pub(crate) fn on_unknown(&mut self, intent: &Intent, now_ms: u64) {
        let hold = |slot: Slot| Slot { order_id: None, acked_ms: now_ms, unknown: true, ..slot };
        match self.take_in_flight(intent) {
            Some(InFlight::Place(new)) => self.set_slot(hold(new)),
            Some(InFlight::Amend { old, new }) => {
                self.add_stray(old.cancel(&self.settings.symbol), now_ms);
                self.set_slot(hold(new));
            }
            Some(InFlight::Cancel { slot, .. }) => self.set_slot(slot),
            Some(InFlight::Stray(stray)) => self.strays.push(stray),
            None => {}
        }
    }

    /// An order update: a terminal one ends its slot or stray, an open one gives its venue id.
    pub(crate) fn on_order(&mut self, order: &Order) {
        let cid = order.client_id.as_deref();
        let oid = order.id.as_str();
        let symbol = self.settings.symbol.clone();
        match order.status {
            OrderStatus::Cancelled | OrderStatus::Rejected | OrderStatus::Filled => {
                for slot in &mut self.slots {
                    if slot.as_ref().is_some_and(|s| names(&s.cancel(&symbol), oid, cid)) {
                        *slot = None;
                    }
                }
                self.strays.retain(|s| !names(&s.cancel, oid, cid));
            }
            OrderStatus::Open | OrderStatus::PartiallyFilled => {
                for slot in self.slots.iter_mut().flatten() {
                    if cid == Some(slot.client_id.as_str())
                        && slot.order_id.is_none()
                        && !oid.is_empty()
                    {
                        slot.order_id = Some(oid.to_string());
                    }
                }
            }
        }
    }

    /// A tick's read of the venue's orders: a slot it lists takes its venue id (an unknown place
    /// rests after all); one it omits is gone, and so is a stray, once the read started at least
    /// [`SETTLE_MS`] after what it asks about.
    pub(crate) fn on_read(&mut self, open: &[Order], view: &View<'_>) {
        self.last_read_ms = Some(view.now_ms);
        let Some(read) = Self::bullet(view).and_then(|e| e.read_started_ms) else { return };
        let listed =
            |c: &CancelOrder| open.iter().find(|o| names(c, &o.id, o.client_id.as_deref()));
        let settled = |since: u64| read >= since.saturating_add(SETTLE_MS);
        let symbol = self.settings.symbol.clone();
        for slot in &mut self.slots {
            let Some(s) = slot.as_mut() else { continue };
            match listed(&s.cancel(&symbol)) {
                Some(o) => {
                    s.order_id = Some(o.id.clone());
                    s.unknown = false;
                }
                None if settled(s.acked_ms) => *slot = None,
                None => {}
            }
        }
        self.strays.retain(|s| listed(&s.cancel).is_some() || !settled(s.since_ms));
    }

    /// The pull's answer: every quote of ours is gone, or it is asked again later. An owner's
    /// pause waiting for it is answered now.
    pub(crate) fn pulled(&mut self, landed: bool, view: &View<'_>) -> Vec<Intent> {
        self.pull.asked = false;
        if landed {
            self.slots = [None, None];
            self.strays.clear();
            self.pull.failed_ms = None;
            self.forced = true;
        } else {
            self.pull.failed_ms = Some(view.now_ms);
        }
        let held = self.position(view);
        self.pull
            .pause_reply
            .take()
            .map(|id| Intent::Reply { id, reply: super::control::pause_reply(landed, held) })
            .into_iter()
            .collect()
    }
}
