#![expect(clippy::arithmetic_side_effects, reason = "a test model: small counters and decimals")]
//! A model venue for the property test: it answers the ladder's intents after random delays, fills
//! and cancels orders on its own, delivers late trades, and checks the invariants from what the
//! ladder has been told (never from the ladder's own bookkeeping).

use std::collections::BTreeMap;

use bb_strategy_sdk::testing::{Market, book, outcome};
use bb_strategy_sdk::{
    Decimal, Event, Intent, NewOrder, Order, OrderStatus, Side, StrategyAdapter, Trade,
};

use crate::{Ladder, LadderSettings, LevelState};

/// splitmix64: a seeded generator, so a failing sequence is replayed from its seed.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next().checked_rem(n).unwrap_or(0)
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// What delivering an event tells the ladder about an order.
#[derive(Clone)]
pub enum Told {
    Nothing,
    Trade(String, Decimal),
    Ended(String, Decimal),
    PlaceOk(String),
    PlaceRefused(String),
    CancelRefused(String),
    /// A fresh venue read, begun at this time, that lists these client ids.
    Read(u64, Vec<String>),
}

struct Queued {
    due: u64,
    event: Event,
    told: Told,
}

/// An order as the model venue and the model of the ladder's knowledge see it.
#[expect(clippy::struct_excessive_bools, reason = "a test model: each flag is one fact it tracks")]
struct MOrder {
    order: NewOrder,
    level: String,
    venue_live: bool,
    true_filled: Decimal,
    told_traded: Decimal,
    told_reported: Decimal,
    told_gone: bool,
    asks: Vec<u64>,
    refusals: Vec<u64>,
    /// Refused with an unreadable error although it had committed: it rests and trades.
    committed_refusal: bool,
    /// When it was placed, and whether the ladder was told it was taken.
    sent_ms: u64,
    told_ok: bool,
}

impl MOrder {
    fn known_fill(&self) -> Decimal {
        self.told_traded.max(self.told_reported)
    }
}

#[derive(Default)]
struct MLevel {
    cycle: Vec<String>,
    counted: bool,
    limit_refused: bool,
}

pub struct Model {
    pub ladder: Ladder,
    pub market: Market,
    pub rng: Rng,
    pub log: Vec<String>,
    step: u64,
    size: Decimal,
    queue: Vec<Queued>,
    orders: BTreeMap<String, MOrder>,
    levels: BTreeMap<String, MLevel>,
    pub completions: u64,
    /// When false the venue answers everything cooperatively (the drain).
    pub chaos: bool,
    /// The last stretch of a run: more fills, and cancels mostly refused or unanswered, so that a
    /// ladder that forgets to ask again is left with an order overfilling its level.
    pub late: bool,
    /// When the host accepted placements and cancels (its per-minute caps are stateful).
    placed_at: Vec<u64>,
    cancelled_at: Vec<u64>,
}

fn level_of(id: &str) -> String {
    id.rsplit_once('-').map_or_else(|| id.to_owned(), |(l, _)| l.to_owned())
}

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

impl Model {
    pub fn new(seed: u64, settings: LadderSettings) -> Self {
        let size = settings.order_size;
        Self {
            ladder: Ladder::new(settings).unwrap(),
            market: Market::new("BTC-USD", "75", "76", 1_000_000),
            rng: Rng(seed),
            log: Vec::new(),
            step: 0,
            size,
            queue: Vec::new(),
            orders: BTreeMap::new(),
            levels: BTreeMap::new(),
            completions: 0,
            chaos: true,
            late: false,
            placed_at: Vec::new(),
            cancelled_at: Vec::new(),
        }
    }

    fn fail(&self, why: &str) -> ! {
        let tail: Vec<&String> = self.log.iter().rev().take(60).collect();
        panic!("{why}\n-- last events (newest first) --\n{tail:#?}");
    }

    pub fn start(&mut self) {
        let view = self.market.view();
        let intents = self.ladder.start(&view).unwrap();
        self.process(&intents);
    }

    /// Deliver `event` to the ladder (after recording what it tells), then process its reply.
    pub fn deliver(&mut self, event: &Event, told: &Told) {
        self.log.push(format!("t={} {:?}", self.market.now_ms, told_name(told, event)));
        self.learn(told);
        let view = self.market.view();
        let intents = self.ladder.on_event(event, &view);
        self.process(&intents);
        self.check_overfill();
    }

    fn learn(&mut self, told: &Told) {
        match told {
            Told::Trade(id, q) => {
                if let Some(o) = self.orders.get_mut(id) {
                    o.told_traded = o.told_traded.saturating_add(*q);
                    if o.told_traded >= o.order.quantity {
                        o.told_gone = true;
                    }
                }
            }
            Told::Ended(id, reported) => {
                if let Some(o) = self.orders.get_mut(id) {
                    o.told_reported = o.told_reported.max(*reported);
                    o.told_gone = true;
                }
            }
            Told::PlaceRefused(id) => {
                if let Some(o) = self.orders.get_mut(id) {
                    o.told_gone = true;
                }
            }
            Told::CancelRefused(id) => {
                let now = self.market.now_ms;
                if let Some(o) = self.orders.get_mut(id) {
                    o.refusals.push(now);
                }
            }
            Told::PlaceOk(id) => {
                if let Some(o) = self.orders.get_mut(id) {
                    o.told_ok = true;
                }
            }
            // A placement with no answer that a read begun long enough after it omits never
            // landed: the ladder may free its level.
            Told::Read(started, listed) => {
                for (id, o) in &mut self.orders {
                    if !o.told_gone
                        && !o.told_ok
                        && o.asks.is_empty()
                        && !listed.contains(id)
                        && o.sent_ms + crate::UNLISTED_AFTER_MS <= *started
                    {
                        o.told_gone = true;
                    }
                }
            }
            Told::Nothing => {}
        }
        self.count_completions();
    }

    fn known_fill(&self, level: &str) -> Decimal {
        let Some(l) = self.levels.get(level) else { return Decimal::ZERO };
        l.cycle
            .iter()
            .filter_map(|id| self.orders.get(id))
            .fold(Decimal::ZERO, |acc, o| acc.saturating_add(o.known_fill()))
    }

    fn count_completions(&mut self) {
        let names: Vec<String> = self.levels.keys().cloned().collect();
        for name in names {
            let fill = self.known_fill(&name);
            if let Some(l) = self.levels.get_mut(&name)
                && fill >= self.size
                && !l.counted
            {
                l.counted = true;
                self.completions += 1;
            }
        }
    }

    fn later(&mut self, event: Event, told: Told, max_delay: u64) {
        let delay = if self.chaos { self.rng.below(max_delay + 1) } else { 0 };
        self.queue.push(Queued { due: self.step + delay, event, told });
    }

    fn process(&mut self, intents: &[Intent]) {
        for intent in intents {
            match intent {
                Intent::Place(order) => self.on_place(intent, order),
                Intent::Cancel(c) => self.on_cancel(intent, c.client_id.as_deref().unwrap_or("")),
                other => self.fail(&format!("unexpected intent {other:?}")),
            }
        }
    }

    fn on_place(&mut self, intent: &Intent, order: &NewOrder) {
        let id = order.client_id.clone().unwrap_or_default();
        let level = level_of(&id);
        // No Place while an order of the level may still be live (the ladder has not been told).
        for (other, o) in &self.orders {
            // Also never beside an order that was reported refused but committed: the ladder cannot
            // know it rests, so a level with an unreadable refusal takes no more orders.
            if o.level == level && (!o.told_gone || (o.committed_refusal && o.venue_live)) {
                self.fail(&format!("placed {id} while {other} may be live"));
            }
        }
        let known = self.known_fill(&level);
        let qty = order.quantity;
        // A new cycle begins only once the last one's completion was counted; otherwise what is
        // placed is exactly what is left of the level.
        let fresh = self.levels.get(&level).is_none_or(|l| l.cycle.is_empty() || l.counted);
        let want = if fresh { self.size } else { self.size.saturating_sub(known) };
        if qty != want {
            self.fail(&format!("{id}: placed {qty}, the level wants {want} (known fill {known})"));
        }
        let l = self.levels.entry(level.clone()).or_default();
        if fresh {
            l.cycle.clear();
            l.counted = false;
        }
        l.cycle.push(id.clone());
        // The host's order rate cap is stateful: 24 placements a minute.
        let now = self.market.now_ms;
        self.placed_at.retain(|t| now.saturating_sub(*t) < 60_000);
        let over_cap = self.placed_at.len() >= 24;
        let refuse = over_cap || (self.chaos && self.rng.chance(12));
        if !refuse {
            self.placed_at.push(now);
        }
        let mut o = MOrder {
            order: order.clone(),
            level,
            venue_live: !refuse,
            true_filled: Decimal::ZERO,
            told_traded: Decimal::ZERO,
            told_reported: Decimal::ZERO,
            told_gone: false,
            asks: Vec::new(),
            refusals: Vec::new(),
            committed_refusal: false,
            sent_ms: now,
            told_ok: false,
        };
        if refuse {
            let reasons = [
                "a post-only order would cross the book",
                "risk gate: new orders are stopped: the book is stale",
                "risk gate: the order rate limit is reached",
                "risk gate: max position",
                "request failed: http 502",
            ];
            let at = if over_cap { 2 } else { self.rng.below(5) };
            let why = reasons.get(usize::try_from(at).unwrap_or(0)).copied().unwrap_or("x");
            if at >= 3
                && let Some(l) = self.levels.get_mut(&o.level)
            {
                l.limit_refused = true;
            }
            // An unreadable refusal (an HTTP error) may have committed: the order then rests and
            // trades although the ladder was told it was refused.
            o.venue_live = at == 4 && self.chaos && self.rng.chance(50);
            o.committed_refusal = o.venue_live;
            self.orders.insert(id.clone(), o);
            self.later(outcome(intent, Err(why)), Told::PlaceRefused(id), 3);
        } else if self.chaos && self.rng.chance(8) {
            // The call errored after it may have reached the venue: the outcome is unknown, and the
            // order rests or never landed.
            o.venue_live = self.rng.chance(50);
            self.orders.insert(id.clone(), o);
            let unknown = Event::Unknown { intent: intent.clone(), error: "timed out".into() };
            self.later(unknown, Told::Nothing, 3);
        } else {
            self.orders.insert(id.clone(), o);
            self.later(outcome(intent, Ok("venue-id")), Told::PlaceOk(id), 3);
        }
    }

    fn on_cancel(&mut self, intent: &Intent, id: &str) {
        let now = self.market.now_ms;
        let Some(o) = self.orders.get_mut(id) else {
            self.fail(&format!("cancel of unknown order {id}"));
        };
        if o.told_gone {
            self.fail(&format!("cancel of {id}, which the ladder was told is gone"));
        }
        // Backoff: unanswered, 10 s; refused, 1 s doubling to 30 s from the refusal.
        if let Some(prev) = o.asks.last().copied() {
            let refused_since: Vec<u64> =
                o.refusals.iter().copied().filter(|t| *t >= prev).collect();
            let least = match refused_since.last() {
                Some(t) => {
                    let n = u32::try_from(o.refusals.len()).unwrap_or(1).saturating_sub(1).min(5);
                    t.saturating_add((1_000u64 << n).min(30_000))
                }
                None => prev.saturating_add(10_000),
            };
            if now < least {
                self.fail(&format!("cancel of {id} asked at {now}, before the backoff ({least})"));
            }
        }
        o.asks.push(now);
        let live = o.venue_live;
        let order = o.order.clone();
        let true_filled = o.true_filled;
        // The host's cancel cap is stateful too: 12 a minute, the rest refused.
        self.cancelled_at.retain(|t| now.saturating_sub(*t) < 60_000);
        if self.cancelled_at.len() >= 12 {
            self.later(
                outcome(intent, Err("risk gate: the cancel rate limit is reached")),
                Told::CancelRefused(id.into()),
                1,
            );
            return;
        }
        self.cancelled_at.push(now);
        let roll = if self.chaos { self.rng.below(100) } else { 0 };
        // The last stretch mostly refuses or ignores cancels.
        let answered = if self.late { 4 } else { 50 };
        let refused = if self.late { 55 } else { 80 };
        if roll < answered {
            if live {
                self.end_order(id, OrderStatus::Cancelled, true_filled, 3);
            } else {
                self.later(
                    outcome(intent, Err("order not found")),
                    Told::CancelRefused(id.into()),
                    2,
                );
            }
        } else if roll < refused {
            self.later(outcome(intent, Err("venue down")), Told::CancelRefused(id.into()), 2);
        }
        let _ = order;
    }

    /// The venue ends `id` (cancel or fill) and sends its order event, reporting `filled` when the
    /// venue does (Bullet's reports 0).
    fn end_order(&mut self, id: &str, status: OrderStatus, filled: Decimal, delay: u64) {
        let Some(o) = self.orders.get_mut(id) else { return };
        o.venue_live = false;
        let reported = if !self.chaos || self.rng.chance(60) { Decimal::ZERO } else { filled };
        let order = &o.order;
        let event = Event::Order(Order {
            id: "venue-id".into(),
            client_id: order.client_id.clone(),
            symbol: order.symbol.clone(),
            side: order.side,
            order_type: order.order_type,
            price: order.price,
            quantity: order.quantity,
            filled_quantity: reported,
            status,
        });
        let told = Told::Ended(
            id.into(),
            if status == OrderStatus::Filled { order.quantity } else { reported },
        );
        self.later(event, told, delay);
    }

    fn deliver_due(&mut self) {
        loop {
            let due: Vec<usize> =
                (0..self.queue.len()).filter(|&i| self.queue[i].due <= self.step).collect();
            if due.is_empty() {
                return;
            }
            let pick = due[usize::try_from(self.rng.below(due.len() as u64)).unwrap_or(0)];
            let q = self.queue.remove(pick);
            self.deliver(&q.event, &q.told);
        }
    }

    fn book_event(&mut self, kind: u64) {
        self.market.book = Some(match kind {
            0 => bb_strategy_sdk::OrderBook::default(),
            1 => book("76", "75", "10"),
            _ => book("75", "76", "10"),
        });
        self.deliver(&Event::Book, &Told::Nothing);
    }

    /// One random step.
    pub fn step(&mut self) {
        self.step += 1;
        self.market.now_ms += 100 + self.rng.below(3_000);
        if self.late && self.rng.chance(30) {
            self.inject_overfill();
            self.deliver_due();
            return;
        }
        // The last stretch: fills whose trades arrive late, and sweeps and cancels that leave a
        // fresh whole order to be overfilled by them.
        let roll = self.rng.below(100);
        let roll = match roll {
            30..=57 if self.late => 24,
            _ => roll,
        };
        match roll {
            0..=24 | 48..=51 => self.book_event(5),
            25..=27 => self.book_event(0),
            28..=29 => self.book_event(1),
            30..=44 => self.venue_fill(),
            45..=46 => self.venue_reject(),
            47..=47 => self.venue_cancel(),

            52..=57 => {
                // Everything resting is swept at once.
                for id in self.live_ids() {
                    let filled = self.orders.get(&id).map_or(Decimal::ZERO, |o| o.true_filled);
                    self.end_order(&id, OrderStatus::Cancelled, filled, 3);
                }
            }
            58..=67 => self.tick_with_read(),
            68..=75 => {
                self.market.now_ms += 5_000 + self.rng.below(40_000);
                self.book_event(5);
            }
            _ => {}
        }
        self.deliver_due();
    }

    /// The venue turns cooperative and everything is delivered, with time passing and good books.
    pub fn drain(&mut self) {
        self.chaos = false;
        for _ in 0..40 {
            self.step += 1;
            self.market.now_ms += 31_000;
            self.deliver_due();
            self.book_event(5);
            self.deliver_due();
        }
        // A Tick emits nothing once the ladder is laid.
        let view = self.market.view();
        let out = self.ladder.on_event(&Event::Tick, &view);
        if !out.is_empty() {
            self.fail(&format!("a tick emitted {out:?}"));
        }
    }
}

fn told_name(told: &Told, event: &Event) -> String {
    match (told, event) {
        (Told::Nothing, e) => format!("{e:?}").chars().take(60).collect(),
        (Told::Trade(id, q), _) => format!("trade {id} {q}"),
        (Told::Ended(id, f), _) => format!("ended {id} reported {f}"),
        (Told::PlaceOk(id), _) => format!("place ok {id}"),
        (Told::PlaceRefused(id), _) => format!("place refused {id}"),
        (Told::CancelRefused(id), _) => format!("cancel refused {id}"),
        (Told::Read(started, listed), _) => format!("read from {started} lists {listed:?}"),
    }
}

#[path = "prop_checks.rs"]
mod checks;

#[path = "prop_venue.rs"]
mod venue;
