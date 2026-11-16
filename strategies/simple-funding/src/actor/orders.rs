use std::collections::HashMap;
use std::time::Duration;

use bb_strategy_sdk::{NewOrder, OrderBook, OrderType, Side, signed_position};
use bb_strategy_support::ioc::{capped_ioc_price, two_sided_mid, within_bps};
use rust_decimal::Decimal;

use super::{PendingLeg, Sent, SimpleFundingActor, grown, usd};
use crate::adapter::io::{FlowError, Mono, Venue};
use crate::cost::{assess, fills_within};
use crate::limits::{BOOK_DEPTH, MAX_IOC_BPS_FROM_MID, PAIR_TOLERANCE};
use crate::plan::{Action, floor_to_step};

impl SimpleFundingActor {
    // -- orders -----------------------------------------------------------------------

    /// An IOC at a fresh REST touch +/- `slippage_bps x mult`, never further than
    /// [`MAX_IOC_BPS_FROM_MID`] from the book's mid. An opening order needs a live, two-sided,
    /// uncrossed REST book whose mid sits near the mark, and a size that fits inside the cap on
    /// it: otherwise it is not built (a cached book is not a cap). A close may fall back to the
    /// cached book, the mark and then `fallback` (the position's entry price), each only while
    /// it is recent, and is capped around whichever reference it used.
    async fn order(
        &mut self,
        b: &Venue,
        a: &Action,
        mult: Decimal,
        fallback: Decimal,
    ) -> Option<NewOrder> {
        let buy = a.side == Side::Buy;
        let touch = |book: &OrderBook| match a.side {
            Side::Buy => book.best_ask().map(|l| l.price),
            Side::Sell => book.best_bid().map(|l| l.price),
        };
        let fresh_book = b.get_orderbook(&a.symbol, BOOK_DEPTH).await.ok();
        let fresh_mid = fresh_book.as_ref().and_then(two_sided_mid);
        let st = self.syms.get(&a.symbol);
        let recent = |at: Option<Mono>| {
            let limit = Duration::from_secs(self.config.stale_secs);
            at.zip(self.now).is_some_and(|(t, now)| {
                now.checked_duration_since(t).is_some_and(|age| age <= limit)
            })
        };
        let mark = st.and_then(|s| s.mark).filter(|m| *m > Decimal::ZERO);
        if !a.reduce_only {
            let near_mark = match (fresh_mid, mark) {
                (Some(mid), Some(mark)) => within_bps(mid, mark, MAX_IOC_BPS_FROM_MID),
                _ => false,
            };
            if fresh_mid.is_none() || !near_mark {
                tracing::warn!(
                    symbol = %a.symbol, ?fresh_mid, ?mark,
                    "simple-funding: no live, sane two-sided book for this entry, skipped"
                );
                return None;
            }
        }
        let cached_book = st.filter(|s| recent(s.book_at)).and_then(|s| s.book.as_deref());
        // The reference every price is capped around, and the base it is priced from.
        let (reference, base) = if let Some(mid) = fresh_mid {
            (mid, fresh_book.as_ref().and_then(touch).unwrap_or(mid))
        } else if let Some(mid) = cached_book.and_then(two_sided_mid) {
            (mid, cached_book.and_then(touch).unwrap_or(mid))
        } else if let Some(mark) = mark.filter(|_| st.is_some_and(|s| recent(s.mark_at))) {
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
            tracing::warn!(
                symbol = %a.symbol, qty = %a.qty, %price,
                "simple-funding: the book cannot fill this entry inside the IOC cap, skipped"
            );
            return None;
        }
        Some(NewOrder {
            protect: None,
            symbol: a.symbol.clone(),
            side: a.side,
            order_type: OrderType::Market,
            price,
            quantity: a.qty,
            client_id: Some(self.client_ids.issue()),
            reduce_only: a.reduce_only,
        })
    }

    /// Send one order. An opening order first records its symbol as the bot's own, and is not
    /// sent if that cannot be made durable.
    async fn submit(&mut self, b: &Venue, order: NewOrder) -> Sent {
        if !order.reduce_only && !self.claim(&order.symbol).await {
            tracing::error!(symbol = %order.symbol, "simple-funding: day state unwritable, entry not placed");
            return Sent::Rejected;
        }
        if !order.reduce_only && self.held.get(&order.symbol).is_none_or(Decimal::is_zero) {
            // A new position: remember what it costs to open, from the book it is opened on.
            let notional = self.config.notional_per_symbol;
            if let Some(st) = self.syms.get_mut(&order.symbol) {
                st.entry_cost =
                    st.book.as_deref().and_then(|bk| assess(bk, st.fee_bps, notional).round_trip);
            }
        }
        tracing::info!(
            symbol = %order.symbol, side = ?order.side, qty = %order.quantity,
            price = %order.price, reduce_only = order.reduce_only, "simple-funding: placing IOC"
        );
        match b.place_orders(std::slice::from_ref(&order)).await {
            Ok(results) => match results.first() {
                Some(r) if r.success => Sent::Took,
                Some(r) => {
                    tracing::warn!(symbol = %order.symbol, error = ?r.error, "simple-funding: order rejected");
                    Sent::Rejected
                }
                None => self.ambiguous(&order, "the venue answered with no result"),
            },
            // The order was refused before it left this process.
            Err(e @ (FlowError::Strategy(_) | FlowError::Config(_))) => {
                tracing::warn!(symbol = %order.symbol, error = %e, "simple-funding: order not sent");
                Sent::Rejected
            }
            // Anything else may have reached the venue: it is not known to have failed.
            Err(e) => self.ambiguous(&order, &e.to_string()),
        }
    }

    fn ambiguous(&self, order: &NewOrder, why: &str) -> Sent {
        let text = format!(
            "simple-funding: {} {:?} {} may or may not have filled ({why}); reconciling from the \
             venue's position before any further entry",
            order.symbol, order.side, order.quantity
        );
        tracing::error!("{text}");
        self.io.raise("simple_funding_ambiguous_order", text);
        Sent::Ambiguous
    }

    /// Build and send one action.
    async fn place_one(
        &mut self,
        b: &Venue,
        a: &Action,
        mult: Decimal,
        fallback: &HashMap<String, Decimal>,
    ) -> Sent {
        let fb = fallback.get(&a.symbol).copied().unwrap_or(Decimal::ZERO);
        let Some(order) = self.order(b, a, mult, fb).await else {
            tracing::warn!(symbol = %a.symbol, "simple-funding: no order built, skipped");
            return Sent::Rejected;
        };
        let (price, leg) = (order.price, self.leg_of(&order));
        let sent = self.submit(b, order).await;
        if sent == Sent::Ambiguous {
            // A lone opening order that may have filled: it has no sibling, so whatever it
            // grew by is unmatched until the venue says otherwise.
            if let Some(leg) = leg {
                self.hold_pending(vec![PendingLeg { price, ..leg }]);
            }
        }
        sent
    }

    /// The pending-leg record of an opening `order` (`None` for a close).
    fn leg_of(&self, order: &NewOrder) -> Option<PendingLeg> {
        (!order.reduce_only).then(|| PendingLeg {
            symbol: order.symbol.clone(),
            side: order.side,
            before: self.held.get(&order.symbol).copied().unwrap_or(Decimal::ZERO),
            price: order.price,
        })
    }

    /// Place `actions`: exits first, then the entries as short/long pairs. Returns how many
    /// orders the venue took (or may have).
    pub(super) async fn execute(
        &mut self,
        b: &Venue,
        actions: &[Action],
        mult: Decimal,
        fallback: &HashMap<String, Decimal>,
    ) -> usize {
        let mut placed = 0;
        for a in actions.iter().filter(|a| a.reduce_only) {
            placed += usize::from(self.place_one(b, a, mult, fallback).await != Sent::Rejected);
        }
        let sells: Vec<&Action> =
            actions.iter().filter(|a| !a.reduce_only && a.side == Side::Sell).collect();
        let buys: Vec<&Action> =
            actions.iter().filter(|a| !a.reduce_only && a.side == Side::Buy).collect();
        for (short, long) in sells.iter().zip(&buys) {
            placed += self.enter_pair(b, short, long, mult, fallback).await;
        }
        let paired = sells.len().min(buys.len());
        for a in sells.iter().skip(paired).chain(buys.iter().skip(paired)) {
            placed += usize::from(self.place_one(b, a, mult, fallback).await != Sent::Rejected);
        }
        placed
    }

    /// The signed size of `symbol` now; `None` when the venue cannot be read.
    async fn signed_now(&self, b: &Venue, symbol: &str) -> Option<Decimal> {
        let positions = b.get_positions().await.ok()?;
        Some(signed_position(&positions, symbol))
    }

    /// Open `first` and `second` together. The second is sized to what the first actually
    /// filled, and the first is trimmed (or closed) by whatever the second leaves short, so a
    /// rejected or clipped sibling never leaves a one-sided book. An ambiguous submit (timeout,
    /// no answer) is read back from the venue: a leg that did commit is treated as filled, one
    /// the venue does not show yet is held as pending and reconciled before any further entry.
    /// Returns orders taken (or possibly taken).
    async fn enter_pair(
        &mut self,
        b: &Venue,
        first: &Action,
        second: &Action,
        mult: Decimal,
        fallback: &HashMap<String, Decimal>,
    ) -> usize {
        let fb = |a: &Action| fallback.get(&a.symbol).copied().unwrap_or(Decimal::ZERO);
        let Some(order_a) = self.order(b, first, mult, fb(first)).await else {
            tracing::warn!(pair = ?(&first.symbol, &second.symbol), "simple-funding: pair skipped, no order for the first leg");
            return 0;
        };
        let Some(order_b) = self.order(b, second, mult, fb(second)).await else {
            tracing::warn!(pair = ?(&first.symbol, &second.symbol), "simple-funding: pair skipped, no order for the second leg");
            return 0;
        };
        let before_a = self.held.get(&first.symbol).copied().unwrap_or(Decimal::ZERO);
        let before_b = self.held.get(&second.symbol).copied().unwrap_or(Decimal::ZERO);
        let price_a = order_a.price;
        let price_b = order_b.price;
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

        let sent_a = self.submit(b, order_a).await;
        if sent_a == Sent::Rejected {
            return 0;
        }
        let Some(after_a) = self.signed_now(b, &first.symbol).await else {
            tracing::error!(symbol = %first.symbol, "simple-funding: first leg's fill unreadable, sibling not placed; reconciling from the venue");
            self.hold_pending(vec![leg_a]);
            return 1;
        };
        let filled_a = grown(first.side, before_a, after_a);
        if filled_a <= Decimal::ZERO {
            if sent_a == Sent::Ambiguous {
                // Not visible yet: it may still land. No sibling, no new entry until it is settled.
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
            self.unwind(b, first, filled_a).await;
            return 1;
        }
        let order_b = NewOrder { quantity: qty_b, ..order_b };
        let sent_b = self.submit(b, order_b).await;
        if sent_b == Sent::Rejected {
            self.unwind(b, first, filled_a).await;
            return 1;
        }
        let Some(after_b) = self.signed_now(b, &second.symbol).await else {
            // Neither leg's final size is known: confirm both from the venue, or unwind.
            tracing::error!(symbol = %second.symbol, "simple-funding: second leg's fill unreadable; reconciling both legs from the venue");
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
            self.unwind(b, first, qty).await;
        }
        2
    }
}

mod flatten;
mod unwind;
