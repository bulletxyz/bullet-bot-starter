use bb_strategy_sdk::{BULLET, Intent, Order, OrderBook, OrderStatus, View};

use rust_decimal::Decimal;

use crate::adapter::{GridStrategy, Seen};
use crate::levels::{FillOutcome, LevelState};

impl GridStrategy {
    pub(super) fn see(&mut self, view: &View<'_>) {
        self.age_note(view.now_ms);
        let market = self.market(view);
        self.seen = Seen {
            mid: self.book(view).and_then(OrderBook::midpoint).or(self.seen.mid),
            position: self.position(view),
            avg_entry: market.and_then(|m| m.avg_entry),
            realized: market.map_or(Decimal::ZERO, |m| m.realized_pnl),
        };
    }

    pub(super) fn on_fill(
        &mut self,
        event: &bb_strategy_sdk::Trade,
        view: &View<'_>,
    ) -> Vec<Intent> {
        if let Some(cid) = &self.entry
            && event.client_id.as_deref() == Some(cid.as_str())
        {
            tracing::info!(qty = %event.quantity, price = %event.price, "Grid starting position filled");
            return Vec::new();
        }
        let Some(idx) = self.state.find_fill_target(event.client_id.as_deref(), &event.order_id)
        else {
            tracing::debug!(order_id = %event.order_id, "Trade with no matching level — ignoring");
            return Vec::new();
        };
        tracing::info!(
            level = idx,
            side = %event.side,
            price = %event.price,
            qty = %event.quantity,
            net_pos = %self.position(view),
            "Grid fill"
        );
        if let FillOutcome::Complete { rearm: Some((to, side)) } =
            self.state.record_fill(idx, event.quantity, self.order_size())
        {
            tracing::info!(from = idx, to, side = %side, "Re-armed adjacent level");
        }
        self.place_pending_orders(view)
    }

    pub(super) fn on_order(&mut self, order: &Order) {
        // Learn exchange order_id once the venue acks our placement.
        if let Some(cid) = order.client_id.as_deref()
            && let Some(level) =
                self.state.levels.iter_mut().find(|l| l.client_id.as_deref() == Some(cid))
            && level.order_id.is_none()
            && !order.id.is_empty()
        {
            level.order_id = Some(order.id.clone());
        }
        // Cancelled/rejected → flip back to Pending so the tick re-places.
        let cid = order.client_id.as_deref();
        let oid = order.id.as_str();
        if matches!(order.status, OrderStatus::Cancelled | OrderStatus::Rejected)
            && let Some(level) = self.state.levels.iter_mut().find(|l| {
                (cid.is_some() && l.client_id.as_deref() == cid)
                    || (!oid.is_empty() && l.order_id.as_deref() == Some(oid))
            })
            && level.state == LevelState::Active
        {
            tracing::info!(
                level = level.index,
                price = %level.price,
                side = ?level.side,
                status = ?order.status,
                "Lifecycle event flipped Active level back to Pending"
            );
            level.state = LevelState::Pending;
            level.client_id = None;
            level.order_id = None;
        }
    }

    pub(super) fn on_book(&mut self, view: &View<'_>) -> Vec<Intent> {
        self.check_halt(view, false).unwrap_or_default()
    }

    pub(super) fn on_tick(&mut self, view: &View<'_>) -> Vec<Intent> {
        if let Some(mut halted) = self.check_halt(view, true) {
            self.ask_pull_again(&mut halted);
            return halted;
        }
        let mut intents = Vec::new();
        if let Some(mid) = self.book(view).and_then(OrderBook::midpoint) {
            intents.extend(self.handle_range(mid, self.position(view), view.now_ms));
        }
        intents.extend(self.handle_trend_filter(view));
        // With no venue read this tick, the levels keep their state: a stale list would show
        // every level as gone and the grid would place it all again.
        let bullet = view.exchange(BULLET);
        if let Some(live) = bullet.and_then(|b| b.open_orders.as_deref()) {
            let read_ms = bullet.and_then(|b| b.read_started_ms).unwrap_or(view.now_ms);
            let missing = self.reconcile_missing_orders(live, read_ms);
            if missing > 0 {
                tracing::warn!(missing, "Detected missing grid orders");
            }
            intents.extend(self.cancel_strays(live, read_ms));
        }
        intents.extend(self.place_pending_orders(view));
        tracing::info!(
            active = self.state.active_count(),
            net_pos = %self.position(view),
            pnl = %self.seen.realized,
            paused = self.paused(),
            "Grid tick"
        );
        intents
    }
}
