//! Building and sending the IOC orders: priced off a live book, capped, one at a time.

#![expect(clippy::arithmetic_side_effects, reason = "order counts are tiny")]

use std::collections::BTreeMap;

use bb_strategy_sdk::{Answer, Decimal, NewOrder, OrderType, Side, VenueCall};
use bb_strategy_support::ioc::{capped_ioc_price, two_sided_mid, within_bps};

use super::{Core, PendingLeg, Sent, UNIVERSE};
use crate::cost::{assess, fills_within};
use crate::limits::MAX_IOC_BPS_FROM_MID;
use crate::plan::Action;

impl Core {
    /// An IOC at a fresh touch +/- `slippage_bps x mult`, never further than
    /// [`MAX_IOC_BPS_FROM_MID`] from the book's mid. An opening order needs a live, two-sided,
    /// uncrossed book whose mid sits near the mark, and a size that fits inside the cap on it:
    /// otherwise it is not built (a cached book is not a cap). A close may fall back to the cached
    /// book, the mark and then `fallback` (the position's entry price), each only while recent,
    /// and is capped around whichever reference it used.
    async fn order(&mut self, a: &Action, mult: Decimal, fallback: Decimal) -> Option<NewOrder> {
        let buy = a.side == Side::Buy;
        let touch = |book: &bb_strategy_sdk::OrderBook| match a.side {
            Side::Buy => book.best_ask().map(|l| l.price),
            Side::Sell => book.best_bid().map(|l| l.price),
        };
        let fresh_book = self.fresh_book(&a.symbol).await;
        let fresh_mid = fresh_book.as_ref().and_then(two_sided_mid);
        let now = self.now_ms();
        let limit = self.config.stale_secs.saturating_mul(1_000);
        let recent = |at: Option<u64>| {
            at.is_some_and(|t| now.checked_sub(t).is_some_and(|age| age <= limit))
        };
        let seen = self.io.with(|s| s.seen.get(&a.symbol).cloned()).unwrap_or_default();
        let mark = seen.mark.filter(|m| *m > Decimal::ZERO);
        if !a.reduce_only {
            let near_mark = match (fresh_mid, mark) {
                (Some(mid), Some(mark)) => within_bps(mid, mark, MAX_IOC_BPS_FROM_MID),
                _ => false,
            };
            if fresh_mid.is_none() || !near_mark {
                return None;
            }
        }
        let cached_book = seen.book.as_ref().filter(|_| recent(seen.book_at_ms));
        let (reference, base) = if let Some(mid) = fresh_mid {
            (mid, fresh_book.as_ref().and_then(touch).unwrap_or(mid))
        } else if let Some(mid) = cached_book.and_then(two_sided_mid) {
            (mid, cached_book.and_then(touch).unwrap_or(mid))
        } else if let Some(mark) = mark.filter(|_| recent(seen.mark_at_ms)) {
            (mark, mark)
        } else if fallback > Decimal::ZERO {
            (fallback, fallback)
        } else {
            return None;
        };
        let slip = self.config.slippage_bps.checked_mul(mult)?;
        let price = capped_ioc_price(a.side, base, reference, slip, MAX_IOC_BPS_FROM_MID)?;
        if !a.reduce_only
            && !fresh_book.as_ref().is_some_and(|bk| fills_within(bk, buy, a.qty, price))
        {
            return None;
        }
        Some(NewOrder {
            symbol: a.symbol.clone(),
            side: a.side,
            order_type: OrderType::Market,
            price,
            quantity: a.qty,
            client_id: Some(self.client_ids.issue()),
            reduce_only: a.reduce_only,
            protect: None,
        })
    }

    /// Send one order. An opening order first records its symbol as its own.
    pub(crate) async fn submit(&mut self, order: NewOrder) -> Sent {
        if !order.reduce_only {
            self.owned.insert(order.symbol.clone());
            if self.held.get(&order.symbol).is_none_or(Decimal::is_zero) {
                // A new position: remember what it costs to open, from the book it is opened on.
                let notional = self.config.notional_per_symbol;
                let book = self.io.with(|s| s.seen.get(&order.symbol).and_then(|m| m.book.clone()));
                if let Some(st) = self.syms.get_mut(&order.symbol) {
                    st.entry_cost =
                        book.and_then(|bk| assess(&bk, st.fee_bps, notional).round_trip);
                }
            }
        }
        let call = VenueCall::Place(vec![order]);
        match self.io.call_on(UNIVERSE, call).await {
            Ok(Answer::Placed(results)) => match results.first() {
                Some(r) if r.success => Sent::Took,
                Some(_) => Sent::Rejected,
                None => Sent::Ambiguous,
            },
            // Refused before it left the host.
            Err(e) if e.never_sent => Sent::Rejected,
            // Anything else (an error, an answer that is not a result) may have reached the venue:
            // it is not known to have failed.
            Ok(_) | Err(_) => Sent::Ambiguous,
        }
    }

    /// Build and send one action.
    pub(crate) async fn place_one(
        &mut self,
        a: &Action,
        mult: Decimal,
        fallback: &BTreeMap<String, Decimal>,
    ) -> Sent {
        let fb = fallback.get(&a.symbol).copied().unwrap_or(Decimal::ZERO);
        let Some(order) = self.order(a, mult, fb).await else { return Sent::Rejected };
        let (price, leg) = (order.price, self.leg_of(&order));
        let sent = self.submit(order).await;
        if sent == Sent::Ambiguous {
            // A lone opening order that may have filled has no sibling: whatever it grew by is
            // unmatched until the venue says otherwise.
            if let Some(leg) = leg {
                self.hold_pending(vec![PendingLeg { price, ..leg }]);
            }
        }
        sent
    }

    /// The pending-leg record of an opening `order` (`None` for a close).
    pub(crate) fn leg_of(&self, order: &NewOrder) -> Option<PendingLeg> {
        (!order.reduce_only).then(|| PendingLeg {
            symbol: order.symbol.clone(),
            side: order.side,
            before: self.held.get(&order.symbol).copied().unwrap_or(Decimal::ZERO),
            price: order.price,
        })
    }

    /// Build the orders of a pair's two legs.
    pub(crate) async fn pair_orders(
        &mut self,
        first: &Action,
        second: &Action,
        mult: Decimal,
        fallback: &BTreeMap<String, Decimal>,
    ) -> Option<(NewOrder, NewOrder)> {
        let fb = |a: &Action| fallback.get(&a.symbol).copied().unwrap_or(Decimal::ZERO);
        let order_a = self.order(first, mult, fb(first)).await?;
        let order_b = self.order(second, mult, fb(second)).await?;
        Some((order_a, order_b))
    }

    /// Place `actions`: exits first, then the entries as short/long pairs. Returns how many orders
    /// the venue took (or may have).
    pub(crate) async fn execute(
        &mut self,
        actions: &[Action],
        mult: Decimal,
        fallback: &BTreeMap<String, Decimal>,
    ) -> usize {
        let mut placed = 0;
        for a in actions.iter().filter(|a| a.reduce_only) {
            placed += usize::from(self.place_one(a, mult, fallback).await != Sent::Rejected);
        }
        let sells: Vec<&Action> =
            actions.iter().filter(|a| !a.reduce_only && a.side == Side::Sell).collect();
        let buys: Vec<&Action> =
            actions.iter().filter(|a| !a.reduce_only && a.side == Side::Buy).collect();
        for (short, long) in sells.iter().zip(&buys) {
            placed += self.enter_pair(short, long, mult, fallback).await;
        }
        let paired = sells.len().min(buys.len());
        for a in sells.iter().skip(paired).chain(buys.iter().skip(paired)) {
            placed += usize::from(self.place_one(a, mult, fallback).await != Sent::Rejected);
        }
        placed
    }
}
