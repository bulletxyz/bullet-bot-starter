use std::collections::HashSet;

use bb_strategy_sdk::Order;

use crate::adapter::{GridStrategy, SETTLE_MS};
use crate::levels::LevelState;

impl GridStrategy {
    /// Missing-order reconcile: any `Active` level whose order is not among the
    /// venue's live orders (e.g., silently cancelled by a venue edge case)
    /// flips back to `Pending` so we re-place next tick.
    pub(in crate::adapter) fn reconcile_missing_orders(
        &mut self,
        live: &[Order],
        read_ms: u64,
    ) -> usize {
        let live_oids: HashSet<&str> = live.iter().map(|o| o.id.as_str()).collect();
        let live_client_ids: HashSet<&str> =
            live.iter().filter_map(|o| o.client_id.as_deref()).collect();

        let mut missing = 0;
        for l in &mut self.state.levels {
            if l.state != LevelState::Active {
                continue;
            }
            let is_live = match (&l.client_id, &l.order_id) {
                (Some(cid), _) if live_client_ids.contains(cid.as_str()) => true,
                (_, Some(oid)) if live_oids.contains(oid.as_str()) => true,
                _ => false,
            };
            let held = l.client_id.as_ref().and_then(|c| self.placed_at.get(c)).copied();
            if is_live {
                // It turned out to rest: no longer held.
                if let Some(c) = &l.client_id {
                    self.placed_at.remove(c);
                }
            }
            if !is_live && held.is_some_and(|at| read_ms < at.saturating_add(SETTLE_MS)) {
                // A read just after the place's answer may not list it yet (REST lag): held.
                continue;
            }
            if !is_live {
                // Its order may still rest behind a lagging read: let go of, not forgotten.
                if let Some(c) = l.client_id.take() {
                    self.placed_at.remove(&c);
                    self.let_go.insert(c, read_ms);
                }
                l.state = LevelState::Pending;
                l.order_id = None;
                missing += 1;
            }
        }
        // An answer whose level has since let go of its order holds nothing.
        let held: HashSet<&str> =
            self.state.levels.iter().filter_map(|l| l.client_id.as_deref()).collect();
        self.placed_at.retain(|c, _| held.contains(c.as_str()));
        missing
    }
}
