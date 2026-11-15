//! A pair entry: the second leg is sized to what the first filled, and the first is trimmed or
//! closed by whatever the second leaves short.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal USD notionals and sizes sit far below 7.9e28; counts are small"
)]

use std::collections::BTreeMap;

use bb_strategy_sdk::{Decimal, NewOrder};

use super::{Core, PendingLeg, Sent, grown, usd};
use crate::limits::PAIR_TOLERANCE;
use crate::plan::{Action, floor_to_step};

impl Core {
    /// The signed size of `symbol` now; `None` when the venue cannot be read.
    pub(crate) async fn signed_now(&self, symbol: &str) -> Option<Decimal> {
        let positions = self.positions().await.ok()?;
        Some(bb_strategy_sdk::signed_position(&positions, symbol))
    }

    /// Open `first` and `second` together. A rejected or clipped sibling never leaves a one-sided
    /// book. An ambiguous submit is read back from the venue: a leg that did commit is treated as
    /// filled, one the venue does not show yet is held as pending and reconciled before any
    /// further entry. Returns orders taken (or possibly taken).
    pub(crate) async fn enter_pair(
        &mut self,
        first: &Action,
        second: &Action,
        mult: Decimal,
        fallback: &BTreeMap<String, Decimal>,
    ) -> usize {
        let Some((order_a, order_b)) = self.pair_orders(first, second, mult, fallback).await else {
            return 0;
        };
        let before_a = self.held.get(&first.symbol).copied().unwrap_or(Decimal::ZERO);
        let before_b = self.held.get(&second.symbol).copied().unwrap_or(Decimal::ZERO);
        let (price_a, price_b) = (order_a.price, order_b.price);
        let leg_a = PendingLeg {
            symbol: first.symbol.clone(),
            side: first.side,
            before: before_a,
            price: price_a,
        };
        let leg_b = PendingLeg {
            symbol: second.symbol.clone(),
            side: second.side,
            before: before_b,
            price: price_b,
        };

        let sent_a = self.submit(order_a).await;
        if sent_a == Sent::Rejected {
            return 0;
        }
        let Some(after_a) = self.signed_now(&first.symbol).await else {
            // The first leg's fill is unreadable: no sibling; reconcile from the venue.
            self.hold_pending(vec![leg_a]);
            return 1;
        };
        let filled_a = grown(first.side, before_a, after_a);
        if filled_a <= Decimal::ZERO {
            if sent_a == Sent::Ambiguous {
                // Not visible yet: it may still land. No sibling, no new entry until settled.
                self.hold_pending(vec![leg_a]);
            }
            return 1;
        }
        let usd_a = usd(filled_a, price_a);
        let step_b = self.syms.get(&second.symbol).and_then(|s| s.step);
        let qty_b = if usd_a >= usd(second.qty, price_b) {
            second.qty
        } else {
            let raw = usd_a.checked_div(price_b).unwrap_or(Decimal::ZERO);
            step_b.map_or(raw, |step| floor_to_step(raw, step))
        };
        if qty_b <= Decimal::ZERO {
            self.unwind(first, filled_a).await;
            return 1;
        }
        let order_b = NewOrder { quantity: qty_b, ..order_b };
        let sent_b = self.submit(order_b).await;
        if sent_b == Sent::Rejected {
            self.unwind(first, filled_a).await;
            return 1;
        }
        let Some(after_b) = self.signed_now(&second.symbol).await else {
            // Neither leg's final size is known: confirm both from the venue, or unwind.
            self.hold_pending(vec![leg_a, leg_b]);
            return 2;
        };
        let filled_b = grown(second.side, before_b, after_b);
        if sent_b == Sent::Ambiguous && filled_b <= Decimal::ZERO {
            // The second leg may still land: if it does while the first is gone it is an orphan.
            self.hold_pending(vec![leg_a, leg_b]);
        }
        let short = usd_a - usd(filled_b.max(Decimal::ZERO), price_b);
        if short > usd_a * PAIR_TOLERANCE {
            let qty = if filled_b <= Decimal::ZERO {
                filled_a
            } else {
                let raw = short.checked_div(price_a).unwrap_or(filled_a).min(filled_a);
                self.syms
                    .get(&first.symbol)
                    .and_then(|s| s.step)
                    .map_or(raw, |step| floor_to_step(raw, step))
            };
            self.unwind(first, qty).await;
        }
        2
    }
}
