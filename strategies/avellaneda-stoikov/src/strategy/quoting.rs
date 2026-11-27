//! The ladder: build the target quotes from the model, diff them against the tracked slots and
//! return the cancels, amends and places that bring the venue to them.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal USD notionals, prices, sizes and bps sit far below 7.9e28; constant divisors; \
              slot counts where the subtrahend is a filtered subset"
)]

use bb_strategy_sdk::{
    AmendOrder, BULLET, CancelOrder, ExchangeIntent, Intent, NewOrder, OrderBook, OrderType, Side,
    View,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

use crate::model::{LadderRung, Quote, SpreadBounds, quote_ladder};
use crate::strategy::AvellanedaStoikov;
use crate::strategy::slots::{InFlight, Plan, QuoteSlot};

type Target = (Side, usize, Decimal, Decimal);

impl AvellanedaStoikov {
    #[expect(
        clippy::cast_precision_loss,
        reason = "order_horizon_secs is a config horizon in seconds, far below 2^53"
    )]
    fn build_ladder(&self, mid: Decimal, position: Decimal) -> Option<(Quote, Vec<LadderRung>)> {
        let sigma = self.volatility.sigma()?;
        let mid_f = mid.to_f64()?;
        let gamma = self.config.gamma.to_f64()?;
        let kappa = self.config.kappa.to_f64()?;
        let tau = self.config.order_horizon_secs as f64;
        let max_pos = self.config.max_position.to_f64()?;
        let target = self.config.inventory_target.to_f64().unwrap_or(0.0);
        let pos = position.to_f64()?;
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

    pub(crate) fn should_force_refresh(&self, now_ms: u64) -> bool {
        match self.last_quote_at_ms {
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

    /// Diff `targets` against the tracked slots and compute the minimal set of operations needed
    /// (amend, place, cancel) without sending them.
    fn plan_orders(
        &mut self,
        targets: &[Target],
        mid: Decimal,
        book: Option<&OrderBook>,
        now_ms: u64,
    ) -> Plan {
        let symbol = self.config.symbol.clone();
        let order_type = self.config.order_type;
        let post_only = matches!(order_type, OrderType::PostOnly);
        let mut matched = vec![false; self.slots.len()];
        let mut plan = Plan::default();

        for (side, level, price, size) in targets {
            let level_bps = self.config.amend_threshold_bps
                + self.config.amend_threshold_step_bps * Decimal::from(*level as u64);
            let threshold = mid * level_bps / Decimal::from(10_000);

            if post_only && book.is_some_and(|b| b.would_cross(*side, *price)) {
                tracing::debug!(side = %side, level, price = %price, "skipping would-cross rung");
                if let Some(idx) = self
                    .slots
                    .iter()
                    .enumerate()
                    .find(|(i, s)| !matched[*i] && s.side == *side && s.level == *level)
                    .map(|(i, _)| i)
                {
                    let existing_also_crosses =
                        book.is_some_and(|b| b.would_cross(*side, self.slots[idx].price));
                    if existing_also_crosses {
                        // Leave matched[idx] = false so the cancel sweep at the
                        // end of this function picks it up and removes the
                        // stale resting order from the venue.
                        tracing::debug!(
                            side = %side,
                            level,
                            "existing slot also crosses book — queuing cancel"
                        );
                    } else {
                        matched[idx] = true;
                        plan.kept.push(self.slots[idx].clone());
                    }
                }
                continue;
            }

            let existing = self
                .slots
                .iter()
                .enumerate()
                .find(|(i, s)| !matched[*i] && s.side == *side && s.level == *level)
                .map(|(i, _)| i);

            let new_slot = |client_id: String| QuoteSlot {
                side: *side,
                level: *level,
                price: *price,
                size: *size,
                client_id: Some(client_id),
                order_id: None,
                placed_at_ms: Some(now_ms),
            };
            let new_order = |client_id: String| NewOrder {
                protect: None,
                symbol: symbol.clone(),
                side: *side,
                order_type,
                price: *price,
                quantity: *size,
                client_id: Some(client_id),
                reduce_only: false,
            };

            if let Some(idx) = existing {
                matched[idx] = true;
                let slot = &self.slots[idx];
                if (slot.price - *price).abs() < threshold {
                    plan.kept.push(slot.clone());
                    continue;
                }
                if slot.order_id.is_none() && slot.client_id.is_none() {
                    plan.kept.push(slot.clone());
                    continue;
                }
                let cid = self.client_ids.issue();
                let amend = AmendOrder {
                    cancel: CancelOrder {
                        symbol: symbol.clone(),
                        order_id: slot.order_id.clone().unwrap_or_default(),
                        client_id: slot.client_id.clone(),
                    },
                    new_order: new_order(cid.clone()),
                };
                plan.amends.push((amend, slot.clone(), new_slot(cid)));
            } else {
                let cid = self.client_ids.issue();
                plan.places.push((new_order(cid.clone()), new_slot(cid)));
            }
        }

        for (i, slot) in self.slots.iter().enumerate() {
            if matched[i] || (slot.order_id.is_none() && slot.client_id.is_none()) {
                continue;
            }
            let cancel = CancelOrder {
                symbol: symbol.clone(),
                order_id: slot.order_id.clone().unwrap_or_default(),
                client_id: slot.client_id.clone(),
            };
            plan.cancels.push((cancel, slot.clone()));
        }
        plan
    }

    /// Diff the desired ladder against the tracked slots and ask for the minimal set of
    /// operations to reconcile them:
    /// - slots whose target price is within `amend_threshold_bps` are kept in place,
    /// - slots whose target moved are amended,
    /// - missing rungs are placed,
    /// - unmatched slots (e.g. when inventory caps disable a side) are cancelled.
    ///
    /// Cheap when nothing has moved, so it is safe to call on every book update. The answers come
    /// back as events and settle the slots (`strategy/sync.rs`).
    pub(crate) fn refresh_quotes(&mut self, view: &View<'_>, mid: Decimal) -> Vec<Intent> {
        let now_ms = view.now_ms;
        // Throttle: when ref feeds fire faster than the venue settles, skip refreshes that arrive
        // within the cooldown window. The next event after the window picks up the latest mid.
        let min_interval_ms = self.config.min_refresh_interval_ms;
        if min_interval_ms > 0
            && let Some(last) = self.last_quote_at_ms
            && now_ms.saturating_sub(last) < min_interval_ms
        {
            return Vec::new();
        }
        if self.uses_reference() && self.reference_is_stale(now_ms) {
            tracing::warn!(
                stale_secs = self.config.reference_stale_secs,
                "Reference feed stale — skipping refresh"
            );
            return Vec::new();
        }
        // One round at a time: the last one's answers are not all in yet.
        if !self.in_flight.is_empty() {
            return Vec::new();
        }
        let position = self.position(view);
        let Some((inner, rungs)) = self.build_ladder(mid, position) else {
            tracing::debug!("Not enough volatility samples yet to quote");
            return Vec::new();
        };

        let can_buy = position < self.config.max_position;
        let can_sell = position > -self.config.max_position;

        let mut targets: Vec<Target> = Vec::new();
        for rung in &rungs {
            let size = self.size_for_level(rung.level);
            if can_buy {
                let Some(bid_price) = Decimal::from_f64(rung.bid) else {
                    tracing::error!("Non-finite bid from A-S ladder");
                    return Vec::new();
                };
                targets.push((Side::Buy, rung.level, bid_price, size));
            }
            if can_sell {
                let Some(ask_price) = Decimal::from_f64(rung.ask) else {
                    tracing::error!("Non-finite ask from A-S ladder");
                    return Vec::new();
                };
                targets.push((Side::Sell, rung.level, ask_price, size));
            }
        }

        let plan = self.plan_orders(&targets, mid, self.book(view), now_ms);
        if plan.is_empty() {
            // Everything within threshold and no add/remove churn: last_quote_at stays, so the
            // Tick fallback can still force a refresh once `order_refresh_secs` elapses.
            return Vec::new();
        }

        let Plan { kept, cancels, amends, places } = plan;
        let (n_cancels, n_amends, n_places) = (cancels.len(), amends.len(), places.len());
        let mut intents = Vec::with_capacity(n_cancels + n_amends + n_places);
        self.slots = kept;
        for (cancel, slot) in cancels {
            intents.push(Intent::on(BULLET, ExchangeIntent::Cancel(cancel)));
            self.in_flight.push(InFlight::Cancel(slot));
        }
        for (amend, old, new) in amends {
            intents.push(Intent::on(BULLET, ExchangeIntent::Amend(amend)));
            self.in_flight.push(InFlight::Amend { old, new });
        }
        for (order, new) in places {
            intents.push(Intent::on(BULLET, ExchangeIntent::Place(order)));
            self.in_flight.push(InFlight::Place(new));
        }
        self.round = Some((now_ms, false));

        tracing::info!(
            levels = rungs.len(),
            cancels = n_cancels,
            amends = n_amends,
            places = n_places,
            inner_bid = %Decimal::from_f64(inner.bid).unwrap_or_default(),
            inner_ask = %Decimal::from_f64(inner.ask).unwrap_or_default(),
            reservation = %inner.reservation_price,
            inner_half_spread = %inner.half_spread,
            net_pos = %position,
            "A-S ladder refreshed"
        );
        intents
    }
}
