//! One level of the ladder as a ledger of its orders, and the one function every event goes
//! through ([`Level::apply`]). Nothing else touches a level's accounting.
//!
//! The ledger. A level keeps a bounded list of its orders: client id, size, what trades showed
//! (`traded`), what a cancel reported (`reported`; Bullet's is always 0) and a state. The level's
//! fill is the sum over the orders of its current cycle of the larger of the two, so a trade that
//! arrives after the cancel that reported it is the same fill, and a trade for an earlier order is
//! found by its id and counted where it belongs. Nothing is reset: a cycle ends and the next one
//! begins.
//!
//! The rules.
//! - At most one order of a level is live (not `Gone`), and nothing is placed while one is.
//! - A completion (the level's fill reaches its size) is counted once per cycle. When it has been
//!   counted and nothing is live, the cycle ends and the level is armed again, whole.
//! - An order that would overfill the level (its remainder plus the fill exceed the size) is
//!   cancelled, and the cancel is asked again only on a backoff: after 1 s, doubling to 30 s, while
//!   the venue refuses it, and after 10 s while it is not answered.
//! - A fill that completes the live order places the level again at once; every other placement
//!   (after a refusal, a cancel, a completion that waited for its cancel) waits for a later book
//!   that is usable and does not cross it. A tick does nothing.
//! - A level the venue keeps cancelling ([`MAX_CANCELS`] in [`CANCEL_WINDOW_MS`]) is held.
//! - The list is a hard bound of [`MAX_ORDERS`]. A placement refused for a reason the ladder reads
//!   (a cross, a stop, the rate) never rested and is forgotten at once; an unreadable refusal (an
//!   HTTP error) may have committed, so that order stays and the level takes no more orders. When
//!   the list is full, gone orders go oldest first: those of earlier cycles, then those of this
//!   cycle that filled nothing, then the oldest of this cycle (its fill folded into the level's
//!   carry). The live order is never dropped. A trade for a dropped order is not counted.

use bb_strategy_sdk::{Decimal, OrderBook, Side};

use crate::UNLISTED_AFTER_MS;
use crate::ladder::{LevelState, Rung};

/// Orders a level keeps.
pub(crate) const MAX_ORDERS: usize = 32;
/// Cancels by someone else, per level, that are answered with a new order; one more holds it.
const MAX_CANCELS: usize = 3;
/// How far back those count (ms).
const CANCEL_WINDOW_MS: u64 = 600_000;
/// The order rate limit's window (ms): a refusal for it counts once in it.
const RATE_WINDOW_MS: u64 = 60_000;
/// How long a cancel waits for its answer before it is asked again (ms).
const CANCEL_WAIT_MS: u64 = 10_000;
/// The first wait after a refused cancel (ms), doubling to the longest.
const CANCEL_BACKOFF_MS: u64 = 1_000;
const CANCEL_BACKOFF_MAX_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OrderState {
    /// Sent, no answer yet.
    Pending,
    Resting,
    /// The ladder asked to cancel it; asked again at `next_ms`, after `refusals` refusals.
    CancelAsked {
        next_ms: u64,
        refusals: u32,
    },
    Gone,
}

#[derive(Debug, Clone)]
pub(crate) struct Order {
    pub id: String,
    pub qty: Decimal,
    pub cycle: u64,
    pub traded: Decimal,
    pub reported: Decimal,
    pub state: OrderState,
    /// When it was sent (the bot's clock, ms).
    pub sent_ms: u64,
}

impl Order {
    fn filled(&self) -> Decimal {
        self.traded.max(self.reported)
    }

    fn live(&self) -> bool {
        self.state != OrderState::Gone
    }

    fn left(&self) -> Decimal {
        self.qty.saturating_sub(self.filled())
    }
}

/// Why the venue or the host refused a placement (classified by the caller from the reason).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A post-only order that would cross the book.
    Cross,
    /// New orders are stopped (a stale book, the daily loss): lifts by itself.
    Stopped,
    /// The order rate: clears with time.
    Rate,
    /// A limit or anything else: not tried again.
    Final,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ended {
    Cancelled,
    Rejected,
    Filled,
}

/// What can happen to a level.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Ev<'a> {
    /// The ladder is laid: place the level.
    Lay,
    Trade {
        id: &'a str,
        qty: Decimal,
    },
    /// The venue says the order has ended, having filled `filled` (0 when it does not say).
    Ended {
        id: &'a str,
        by: Ended,
        filled: Decimal,
    },
    PlaceOk {
        id: &'a str,
    },
    PlaceRefused {
        id: &'a str,
        why: Refusal,
    },
    CancelRefused {
        id: &'a str,
    },
    /// A cancel by someone else that the ladder judged to be this level's alone (not a sweep).
    CountCancel,
    /// A book update (the only event that places again or asks again).
    Book,
    /// A fresh venue read, begun at `started_ms`, lists `open`: it settles a pending order
    /// ([`Level::on_read`]).
    Read {
        open: &'a [bb_strategy_sdk::Order],
        started_ms: u64,
    },
}

/// What the caller knows that the level does not.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Cx {
    pub now_ms: u64,
    /// The book is two-sided and not crossed.
    pub usable: bool,
    /// An order at this level's price would cross the book the view holds.
    pub crosses: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    Place { client_id: String, qty: Decimal },
    Cancel { client_id: String },
}

#[derive(Debug, Default)]
pub(crate) struct Out {
    pub actions: Vec<Action>,
    /// Completions counted by this event.
    pub completions: u32,
    /// An order ended by someone else's cancel (the caller judges whether it was a sweep).
    pub venue_cancel: bool,
}

/// Why a level takes no more orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    /// The venue kept cancelling it: held until the bot restarts.
    Held,
    /// Refused with an error the ladder cannot read: an order of it may have committed, so nothing
    /// else is placed on the level (a completion of that order is still counted).
    Dead,
}

#[derive(Debug, Clone)]
pub(crate) struct Level {
    pub side: Side,
    pub index: u32,
    pub price: Decimal,
    size: Decimal,
    orders: Vec<Order>,
    cycle: u64,
    /// The current cycle's completion was counted.
    counted: bool,
    /// The level may be placed again on a later usable book.
    retry: bool,
    /// Why nothing more is placed on the level, if so.
    block: Option<Block>,
    seq: u64,
    /// When someone else cancelled an order of the level (counted), for the cap.
    pub cancels: Vec<u64>,
    rate_counted_ms: Option<u64>,
    /// The fill of orders of the current cycle that were dropped to keep the ledger bounded.
    carry: Decimal,
}

impl Level {
    pub(crate) fn new(side: Side, rank: u32, price: Decimal, order_size: Decimal) -> Self {
        Self {
            side,
            index: rank,
            price,
            size: order_size,
            orders: Vec::new(),
            cycle: 0,
            counted: false,
            retry: false,
            block: None,
            seq: 0,
            cancels: Vec::new(),
            rate_counted_ms: None,
            carry: Decimal::ZERO,
        }
    }

    /// The level's fill in its current cycle.
    pub(crate) fn fill(&self) -> Decimal {
        self.orders
            .iter()
            .filter(|o| o.cycle == self.cycle)
            .fold(self.carry, |acc, o| acc.saturating_add(o.filled()))
    }

    /// Keep the ledger at [`MAX_ORDERS`], a hard bound (a plugin's work and memory are metered):
    /// the live order is never dropped; gone orders go oldest first, those of earlier cycles, then
    /// those of this cycle that filled nothing, then (their fill folded into `carry`, so the
    /// level's fill stays right) the oldest of this cycle. A trade for a dropped order is not
    /// counted.
    fn trim(&mut self) {
        while self.orders.len() > MAX_ORDERS {
            let cycle = self.cycle;
            let at = self
                .orders
                .iter()
                .position(|o| !o.live() && o.cycle < cycle)
                .or_else(|| self.orders.iter().position(|o| !o.live() && o.filled().is_zero()))
                .or_else(|| self.orders.iter().position(|o| !o.live()));
            let Some(at) = at else { break };
            let gone = self.orders.remove(at);
            if gone.cycle == cycle {
                self.carry = self.carry.saturating_add(gone.filled());
            }
        }
    }

    fn live_index(&self) -> Option<usize> {
        self.orders.iter().position(Order::live)
    }

    /// Whether the level has an order of this client id.
    pub(crate) fn owns(&self, id: &str) -> bool {
        self.orders.iter().any(|o| o.id == id)
    }

    /// Whether a cancel the ladder asked for is outstanding.
    #[cfg(test)]
    pub(crate) fn cancel_asked(&self) -> bool {
        self.orders.iter().any(|o| matches!(o.state, OrderState::CancelAsked { .. }))
    }

    /// Whether an order at this level would cross `book`: a buy at or above the best ask, a sell
    /// at or below the best bid. No book, no cross.
    pub(crate) fn crosses(&self, book: Option<&OrderBook>) -> bool {
        let Some(book) = book else { return false };
        match self.side {
            Side::Buy => book.best_ask().is_some_and(|l| self.price >= l.price),
            Side::Sell => book.best_bid().is_some_and(|l| self.price <= l.price),
        }
    }

    /// The level as the report shows it, derived from the ledger.
    pub(crate) fn rung(&self) -> Rung {
        let state = if self.block == Some(Block::Held) {
            LevelState::Held
        } else {
            match self.live_index().and_then(|i| self.orders.get(i)).map(|o| o.state) {
                Some(OrderState::Pending) => LevelState::Pending,
                Some(_) => LevelState::Resting,
                None => LevelState::Refused,
            }
        };
        Rung {
            side: self.side,
            index: self.index,
            price: self.price,
            client_id: self.orders.last().map(|o| o.id.clone()),
            state,
            filled: self.fill(),
        }
    }

    /// The one transition function: `ev` changes the ledger, then the level settles (counts a
    /// completion, cancels an order that would overfill, ends a finished cycle) and acts.
    pub(crate) fn apply(&mut self, ev: Ev<'_>, cx: Cx) -> Out {
        let mut out = Out::default();
        // A fill that completes the live order places the level again at once.
        let mut at_once = false;
        match ev {
            Ev::Lay => {
                self.retry = true;
                at_once = true;
            }
            Ev::Trade { id, qty } => at_once = self.on_trade(id, qty),
            Ev::Ended { id, by, filled } => self.on_ended(id, by, filled, cx, &mut out),
            Ev::PlaceOk { id } => {
                if let Some(o) = self.order_mut(id)
                    && o.state == OrderState::Pending
                {
                    o.state = OrderState::Resting;
                }
            }
            Ev::PlaceRefused { id, why } => self.on_place_refused(id, why, cx),
            Ev::CancelRefused { id } => self.on_cancel_refused(id, cx.now_ms),
            Ev::CountCancel => self.count(cx.now_ms),
            Ev::Read { open, started_ms } => self.on_read(open, started_ms),
            Ev::Book => {}
        }
        self.settle(cx.now_ms, &mut out);
        if matches!(ev, Ev::Book) {
            self.ask_again(cx.now_ms, &mut out);
            if cx.usable && !cx.crosses {
                self.place(cx.now_ms, &mut out);
            }
        } else if at_once {
            self.place(cx.now_ms, &mut out);
        }
        out
    }

    fn order_mut(&mut self, id: &str) -> Option<&mut Order> {
        self.orders.iter_mut().find(|o| o.id == id)
    }

    /// A trade for the order `id`. True when it completed an order that was not waiting for a
    /// cancel (the level is placed again at once).
    fn on_trade(&mut self, id: &str, qty: Decimal) -> bool {
        let Some(o) = self.order_mut(id) else { return false };
        let plain = matches!(o.state, OrderState::Pending | OrderState::Resting);
        o.traded = o.traded.saturating_add(qty);
        if o.live() && o.traded >= o.qty {
            o.state = OrderState::Gone;
            self.retry = true;
            return plain;
        }
        false
    }

    fn on_ended(&mut self, id: &str, by: Ended, filled: Decimal, cx: Cx, out: &mut Out) {
        let Some(o) = self.order_mut(id) else { return };
        if !o.live() {
            // Nothing changes for an order that is already gone, but what the venue says it filled
            // is still a fill (a refused placement that had committed, then was cancelled).
            let reported = if by == Ended::Filled { o.qty } else { filled };
            o.reported = o.reported.max(reported);
            return;
        }
        let own = matches!(o.state, OrderState::CancelAsked { .. });
        let reported = if by == Ended::Filled { o.qty } else { filled };
        o.reported = o.reported.max(reported);
        o.state = OrderState::Gone;
        match by {
            Ended::Cancelled => {
                self.retry = true;
                out.venue_cancel = !own;
            }
            Ended::Rejected => {
                // It never rested: forgotten unless it somehow filled. Placed again on a later
                // book (a reason the ladder cannot read), counted like a rate refusal.
                self.forget_unfilled(id);
                self.retry = true;
                self.count_once_a_window(cx.now_ms);
            }
            Ended::Filled => self.retry = true,
        }
    }

    fn on_place_refused(&mut self, id: &str, why: Refusal, cx: Cx) {
        let Some(o) = self.order_mut(id) else { return };
        if !o.live() {
            return;
        }
        o.state = OrderState::Gone;
        // A refusal the ladder can read (a cross, a stop, the rate) means it never rested: nothing
        // to remember. An unreadable one (an HTTP error) may have committed, so the order stays in
        // the ledger and a late trade for it still counts.
        if why == Refusal::Final {
            self.block = self.block.or(Some(Block::Dead));
        } else {
            self.forget_unfilled(id);
        }
        self.retry = why != Refusal::Final;
        if why == Refusal::Rate {
            self.count_once_a_window(cx.now_ms);
        }
    }

    /// Tried again on a later book, but not for ever: a refusal that clears with time counts once
    /// per rate window toward the cap.
    fn count_once_a_window(&mut self, now_ms: u64) {
        if self.rate_counted_ms.is_none_or(|t| now_ms.saturating_sub(t) >= RATE_WINDOW_MS) {
            self.rate_counted_ms = Some(now_ms);
            self.count(now_ms);
        }
    }

    /// Drop order `id` from the ledger if nothing of it filled.
    fn forget_unfilled(&mut self, id: &str) {
        self.orders.retain(|o| o.id != id || o.filled() > Decimal::ZERO);
    }

    /// A fresh venue read that began at `started_ms` lists `open`. A pending order (sent with no
    /// answer yet, or one whose outcome is unknown) it lists rests. One it omits, sent at least
    /// [`UNLISTED_AFTER_MS`] before the read began, never landed (or has ended, its events not
    /// seen yet): it is gone, and the level is placed again on a later book. It stays in the
    /// ledger, so a late trade for it still counts (and a new order it would overfill is
    /// cancelled).
    fn on_read(&mut self, open: &[bb_strategy_sdk::Order], started_ms: u64) {
        let mut freed = false;
        for o in self.orders.iter_mut().filter(|o| o.state == OrderState::Pending) {
            if open.iter().any(|x| x.client_id.as_deref() == Some(o.id.as_str())) {
                o.state = OrderState::Resting;
            } else if o.sent_ms.saturating_add(UNLISTED_AFTER_MS) <= started_ms {
                o.state = OrderState::Gone;
                freed = true;
            }
        }
        if freed {
            self.retry = true;
        }
    }

    fn on_cancel_refused(&mut self, id: &str, now_ms: u64) {
        let Some(o) = self.order_mut(id) else { return };
        if let OrderState::CancelAsked { refusals, .. } = o.state {
            let refusals = refusals.saturating_add(1);
            let wait = CANCEL_BACKOFF_MS
                .saturating_mul(1u64 << refusals.saturating_sub(1).min(5))
                .min(CANCEL_BACKOFF_MAX_MS);
            o.state = OrderState::CancelAsked { next_ms: now_ms.saturating_add(wait), refusals };
        }
    }

    /// Count a re-placement caused by someone else's cancel (or a rate refusal); past the cap in
    /// the window the level is held.
    fn count(&mut self, now_ms: u64) {
        self.cancels.retain(|t| now_ms.saturating_sub(*t) < CANCEL_WINDOW_MS);
        self.cancels.push(now_ms);
        if self.cancels.len() > MAX_CANCELS {
            self.block = Some(Block::Held);
            self.retry = false;
        }
    }

    /// After every event: count a completion once, cancel an order that would overfill, and end a
    /// finished cycle (armed again, whole) once nothing is live.
    fn settle(&mut self, now_ms: u64, out: &mut Out) {
        let fill = self.fill();
        if fill >= self.size && !self.counted {
            self.counted = true;
            out.completions = out.completions.saturating_add(1);
        }
        let size = self.size;
        match self.live_index().and_then(|i| self.orders.get_mut(i)) {
            Some(o) => {
                if !matches!(o.state, OrderState::CancelAsked { .. })
                    && fill.saturating_add(o.left()) > size
                {
                    o.state = OrderState::CancelAsked {
                        next_ms: now_ms.saturating_add(CANCEL_WAIT_MS),
                        refusals: 0,
                    };
                    out.actions.push(Action::Cancel { client_id: o.id.clone() });
                }
            }
            None if self.counted => {
                self.cycle = self.cycle.saturating_add(1);
                self.carry = Decimal::ZERO;
                self.counted = false;
                self.retry = true;
            }
            None => {}
        }
    }

    /// Ask again for a cancel whose time has come.
    fn ask_again(&mut self, now_ms: u64, out: &mut Out) {
        for o in &mut self.orders {
            if let OrderState::CancelAsked { next_ms, refusals } = o.state
                && next_ms <= now_ms
            {
                o.state = OrderState::CancelAsked {
                    next_ms: now_ms.saturating_add(CANCEL_WAIT_MS),
                    refusals,
                };
                out.actions.push(Action::Cancel { client_id: o.id.clone() });
            }
        }
    }

    /// Place what is left of the level, if it may be placed: nothing live, not held, wanted.
    fn place(&mut self, now_ms: u64, out: &mut Out) {
        if self.block.is_some() || !self.retry || self.live_index().is_some() {
            return;
        }
        let left = self.size.saturating_sub(self.fill());
        if left <= Decimal::ZERO {
            return;
        }
        self.seq = self.seq.saturating_add(1);
        let side = if self.side == Side::Buy { "b" } else { "s" };
        let id = format!("ladder-{side}{}-{}", self.index, self.seq);
        self.orders.push(Order {
            id: id.clone(),
            qty: left,
            cycle: self.cycle,
            traded: Decimal::ZERO,
            reported: Decimal::ZERO,
            state: OrderState::Pending,
            sent_ms: now_ms,
        });
        self.trim();
        self.retry = false;
        out.actions.push(Action::Place { client_id: id, qty: left });
    }
}

#[cfg(test)]
impl Level {
    /// How many orders the ledger holds.
    pub(crate) fn order_count(&self) -> usize {
        self.orders.len()
    }
}
