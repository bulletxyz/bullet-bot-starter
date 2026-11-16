//! What the strategy learns after a round was sent: the answers that settle its slots, the order
//! updates and fills, and the reconcile with the venue's open orders.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal USD notionals, prices, sizes and bps sit far below 7.9e28; constant divisors; \
              slot counts where the subtrahend is a filtered subset"
)]

use std::collections::HashSet;

use bb_strategy_sdk::{CancelOrder, Intent, Order, OrderResult, OrderStatus, Trade};

use crate::strategy::AvellanedaStoikov;
use crate::strategy::slots::{InFlight, QuoteSlot, key_of};

/// How long after an unknown place a venue read must have begun before it is taken as showing the
/// order (the venue's list can lag): the order path's settle margin, as the grid holds it.
pub(crate) const UNKNOWN_SETTLE_MS: u64 = 2_000;

impl AvellanedaStoikov {
    /// The round's last answer is in: a round with no failed call is a refresh, so the next
    /// forced one waits `order_refresh_secs` from it.
    fn settle_round(&mut self) {
        if !self.in_flight.is_empty() {
            return;
        }
        if let Some((at_ms, false)) = self.round.take() {
            self.last_quote_at_ms = Some(at_ms);
        }
        self.round = None;
    }

    fn take_in_flight(&mut self, intent: &Intent) -> Option<InFlight> {
        let key = key_of(intent)?;
        let at = self.in_flight.iter().position(|f| f.key() == key)?;
        Some(self.in_flight.remove(at))
    }

    /// The answer to one of the round's intents (or to an orphan cancel, which keeps no state).
    pub(crate) fn on_outcome(&mut self, intent: &Intent, result: &OrderResult, now_ms: u64) {
        let Some(flight) = self.take_in_flight(intent) else {
            if let Intent::Cancel(_) = intent
                && !result.success
            {
                tracing::warn!(
                    error = result.error.as_deref().unwrap_or("unknown"),
                    "orphan cancel reverted — orphans persist"
                );
            }
            return;
        };
        match flight {
            InFlight::Cancel(slot) => {
                if !result.success {
                    tracing::warn!(
                        side = %slot.side,
                        level = slot.level,
                        error = result.error.as_deref().unwrap_or("unknown"),
                        "unmatched-slot cancel reverted — retaining tracking to retry"
                    );
                    self.slots.push(slot);
                }
            }
            InFlight::Amend { old, new } => {
                if result.success {
                    self.slots.push(acked(new, result, now_ms));
                } else {
                    tracing::warn!(
                        side = %new.side,
                        level = new.level,
                        price = %new.price,
                        error = result.error.as_deref().unwrap_or("unknown"),
                        "Failed to amend A-S quote — retaining old slot"
                    );
                    self.slots.push(old);
                }
            }
            InFlight::Place(new) => {
                if result.success {
                    self.slots.push(acked(new, result, now_ms));
                } else {
                    tracing::warn!(
                        side = %new.side,
                        level = new.level,
                        price = %new.price,
                        error = result.error.as_deref().unwrap_or("unknown"),
                        "Failed to place A-S quote"
                    );
                }
            }
        }
        self.settle_round();
    }

    /// The call that carried out `intent` failed as a whole (a transport error, or one that may
    /// have reached the venue): what the round changed is rolled back, so the next refresh
    /// decides again from the slots it had.
    pub(crate) fn on_not_sent(&mut self, intent: &Intent) {
        let Some(flight) = self.take_in_flight(intent) else { return };
        if let Some((_, failed)) = &mut self.round {
            *failed = true;
        }
        match flight {
            InFlight::Cancel(slot) | InFlight::Amend { old: slot, .. } => self.slots.push(slot),
            InFlight::Place(_) => {}
        }
        self.settle_round();
    }

    /// The call that carried out `intent` errored after it may have reached the venue (a timeout, a
    /// 5xx): a cancel stays tracked, a place or an amend's new order is held as placed (its venue
    /// id unknown) so the level is not placed again. A later read settles it
    /// (`reconcile_with_venue`).
    pub(crate) fn on_unknown(&mut self, intent: &Intent, now_ms: u64) {
        let Some(flight) = self.take_in_flight(intent) else { return };
        if let Some((_, failed)) = &mut self.round {
            *failed = true;
        }
        match flight {
            InFlight::Cancel(slot) => self.slots.push(slot),
            InFlight::Amend { old, new } => {
                self.slots.push(old);
                self.hold(new, now_ms);
            }
            InFlight::Place(new) => self.hold(new, now_ms),
        }
        self.settle_round();
    }

    fn hold(&mut self, slot: QuoteSlot, now_ms: u64) {
        if let Some(cid) = &slot.client_id {
            self.unknown.insert(cid.clone(), now_ms);
        }
        self.slots.push(QuoteSlot { order_id: None, ..slot });
    }

    /// Whether a quote is held for a place whose outcome is still unknown.
    fn holds_unknown(&self) -> bool {
        self.slots.iter().any(|s| {
            s.order_id.is_none()
                && s.client_id.as_ref().is_some_and(|c| self.unknown.contains_key(c))
        })
    }

    pub(crate) fn on_fill(&mut self, event: &Trade) {
        // Drop the filled slot from our local view.
        // Match on (Some(cid) == Some(cid)) or (Some(oid) == Some(oid)).
        // The explicit `Some` gates prevent the naive `a == b` from matching
        // `None == None` — which would otherwise wipe every slot that hasn't
        // yet received its exchange order_id the moment a fill arrives
        // without a client_id (possible on HL for orders placed without cloid).
        let event_cid = event.client_id.as_deref();
        let event_order_id = Some(event.order_id.as_str()).filter(|s| !s.is_empty());
        self.slots.retain(|slot| {
            let cid_match = event_cid.is_some() && slot.client_id.as_deref() == event_cid;
            let order_id_match =
                event_order_id.is_some() && slot.order_id.as_deref() == event_order_id;
            !(cid_match || order_id_match)
        });
        // Force a re-quote on the next tick so both sides follow the new reservation price.
        self.last_quote_at_ms = None;

        tracing::info!(
            side = %event.side,
            price = %event.price,
            qty = %event.quantity,
            "A-S fill"
        );
    }

    pub(crate) fn on_order(&mut self, order: &Order) {
        let cid = order.client_id.as_deref();
        let oid = order.id.as_str();

        match order.status {
            // Terminal: order is gone from the venue's book. Drop our tracking so we don't try to
            // amend it. The Filled case is also handled by the fill handler — both dropping is
            // idempotent.
            OrderStatus::Cancelled | OrderStatus::Rejected | OrderStatus::Filled => {
                self.slots.retain(|s| {
                    let cid_match = cid.is_some() && s.client_id.as_deref() == cid;
                    let order_id_match = !oid.is_empty() && s.order_id.as_deref() == Some(oid);
                    !(cid_match || order_id_match)
                });
            }
            // Place ack: learn the exchange-assigned order_id.
            OrderStatus::Open | OrderStatus::PartiallyFilled => {
                if let Some(cid) = cid
                    && let Some(slot) =
                        self.slots.iter_mut().find(|s| s.client_id.as_deref() == Some(cid))
                    && slot.order_id.is_none()
                    && !oid.is_empty()
                {
                    slot.order_id = Some(oid.to_string());
                }
            }
        }
    }

    /// Whether the slots are due to be compared with the venue's open orders: the stream
    /// reconnected (events during the gap are lost), or the periodic sweep that catches dropped
    /// frames and orphans is due.
    pub(crate) fn reconcile_due(&self, now_ms: u64, reconnected: bool) -> bool {
        reconnected
            || self.holds_unknown()
            || (self.config.reconcile_interval_secs > 0
                && match self.last_reconcile_at_ms {
                    None => true,
                    Some(t) => {
                        now_ms.saturating_sub(t)
                            >= self.config.reconcile_interval_secs.saturating_mul(1000)
                    }
                })
    }

    /// Settle the quotes held for an unknown place against a read of the venue: one it lists takes
    /// its venue id; one it does not list is freed (the level is placed again) only if the read
    /// began at least [`UNKNOWN_SETTLE_MS`] after the unknown, since the venue's list can lag.
    fn settle_unknown(&mut self, open: &[Order], read_started: Option<u64>) {
        let unknown = &mut self.unknown;
        self.slots.retain_mut(|slot| {
            let Some(cid) = slot.client_id.as_deref().filter(|_| slot.order_id.is_none()) else {
                return true;
            };
            let Some(&at) = unknown.get(cid) else { return true };
            if let Some(listed) = open.iter().find(|o| o.client_id.as_deref() == Some(cid)) {
                slot.order_id = Some(listed.id.clone());
                unknown.remove(cid);
                return true;
            }
            let settled = read_started.is_some_and(|r| r >= at.saturating_add(UNKNOWN_SETTLE_MS));
            if settled {
                unknown.remove(cid);
            }
            !settled
        });
        let slots = &self.slots;
        unknown.retain(|c, _| slots.iter().any(|s| s.client_id.as_deref() == Some(c.as_str())));
    }

    /// Bidirectional reconciliation against the venue's actual open orders:
    ///   - Phantom: a slot whose `order_id` isn't in the live list is dropped. The next refresh
    ///     re-places it.
    ///   - Orphan: a live order matching no tracked slot's `order_id` or `client_id` is cancelled.
    ///
    /// Slots whose `order_id` is None (place ack pending) are matched by `client_id` instead, so
    /// legitimate in-flight slots are not dropped. A slot acknowledged after the read began may be
    /// missing from it for that reason alone, and is kept.
    pub(crate) fn reconcile_with_venue(
        &mut self,
        open: &[Order],
        view: &bb_strategy_sdk::View<'_>,
    ) -> Vec<Intent> {
        let live_oids: HashSet<&str> = open.iter().map(|o| o.id.as_str()).collect();
        let live_client_ids: HashSet<&str> =
            open.iter().filter_map(|o| o.client_id.as_deref()).collect();
        let read_started = view.read_started_ms;
        self.settle_unknown(open, read_started);
        let before = self.slots.len();
        self.slots.retain(|s| match s.order_id.as_deref() {
            Some(oid) => {
                live_oids.contains(oid)
                    || s.client_id.as_deref().is_some_and(|c| live_client_ids.contains(c))
                    || s.placed_at_ms.zip(read_started).is_some_and(|(at, read)| at >= read)
            }
            None => true,
        });
        let dropped = before - self.slots.len();

        // Orders alive on the venue that we don't track: match in either direction (oid OR cid) so
        // a slot whose place ack hasn't landed (cid only) still claims its order.
        let tracked_oids: HashSet<&str> =
            self.slots.iter().filter_map(|s| s.order_id.as_deref()).collect();
        let tracked_client_ids: HashSet<&str> =
            self.slots.iter().filter_map(|s| s.client_id.as_deref()).collect();
        let orphans: Vec<&Order> = open
            .iter()
            .filter(|o| {
                let oid_tracked = tracked_oids.contains(o.id.as_str());
                let client_tracked =
                    o.client_id.as_deref().is_some_and(|c| tracked_client_ids.contains(c));
                !oid_tracked && !client_tracked
            })
            .collect();
        if dropped > 0 || !orphans.is_empty() {
            let summary: Vec<String> = orphans
                .iter()
                .map(|o| format!("{}@{}/{}", o.id, o.client_id.as_deref().unwrap_or("-"), o.side))
                .collect();
            tracing::info!(
                phantom_dropped = dropped,
                orphans_found = orphans.len(),
                kept = self.slots.len(),
                live = open.len(),
                orphans = ?summary,
                "reconciled with the venue's open orders"
            );
        }
        self.last_reconcile_at_ms = Some(view.now_ms);
        orphans
            .into_iter()
            .map(|o| {
                Intent::Cancel(CancelOrder {
                    symbol: self.config.symbol.clone(),
                    order_id: o.id.clone(),
                    client_id: o.client_id.clone(),
                })
            })
            .collect()
    }
}

/// `slot` as the venue acknowledged it: with its order id, stamped with the time.
fn acked(mut slot: QuoteSlot, result: &OrderResult, now_ms: u64) -> QuoteSlot {
    slot.order_id.clone_from(&result.order_id);
    slot.placed_at_ms = Some(now_ms);
    slot
}
