//! What the model venue does on its own: fills, rejections, cancels, and the scenario that aims at
//! the ladder's cancel path.

use super::*;

impl Model {
    pub(super) fn live_ids(&self) -> Vec<String> {
        self.orders.iter().filter(|(_, o)| o.venue_live).map(|(i, _)| i.clone()).collect()
    }

    /// The venue fills `quantity` of a live order; the trade reaches the ladder after a delay.
    pub(super) fn venue_fill(&mut self) {
        let live = self.live_ids();
        if live.is_empty() {
            return;
        }
        let id = live[usize::try_from(self.rng.below(live.len() as u64)).unwrap_or(0)].clone();
        let Some(o) = self.orders.get_mut(&id) else { return };
        let left = o.order.quantity.saturating_sub(o.true_filled);
        let steps = left / d("0.001");
        let n = self.rng.below(u64::try_from(steps.trunc().mantissa()).unwrap_or(1).max(1)) + 1;
        let q = (d("0.001") * Decimal::from(n)).min(left);
        if q.is_zero() {
            return;
        }
        o.true_filled = o.true_filled.saturating_add(q);
        let (order, full) = (o.order.clone(), o.true_filled >= o.order.quantity);
        let trade = Trade {
            exchange: "sim".into(),
            symbol: order.symbol.clone(),
            order_id: id.clone(),
            trade_id: None,
            client_id: order.client_id.clone(),
            side: order.side,
            price: order.price,
            quantity: q,
            timestamp: None,
        };
        let delay = if self.late { 14 } else { 6 };
        self.later(on_bullet(SYM, MarketEvent::Fill(trade)), Told::Trade(id.clone(), q), delay);
        if full {
            // Sometimes the Filled event is lost: the trades alone must free the level.
            if self.rng.chance(15) {
                if let Some(o) = self.orders.get_mut(&id) {
                    o.venue_live = false;
                }
            } else {
                self.end_order(&id, OrderStatus::Filled, order.quantity, 6);
            }
        }
    }

    /// The venue rejects an order that has not filled (a reason the ladder cannot read).
    pub(super) fn venue_reject(&mut self) {
        let fresh: Vec<String> = self
            .orders
            .iter()
            .filter(|(_, o)| o.venue_live && o.true_filled.is_zero())
            .map(|(i, _)| i.clone())
            .collect();
        if fresh.is_empty() {
            return;
        }
        let id = fresh[usize::try_from(self.rng.below(fresh.len() as u64)).unwrap_or(0)].clone();
        self.end_order(&id, OrderStatus::Rejected, Decimal::ZERO, 3);
    }

    pub(super) fn venue_cancel(&mut self) {
        let live = self.live_ids();
        if live.is_empty() {
            return;
        }
        let id = live[usize::try_from(self.rng.below(live.len() as u64)).unwrap_or(0)].clone();
        let filled = self.orders.get(&id).map_or(Decimal::ZERO, |o| o.true_filled);
        self.end_order(&id, OrderStatus::Cancelled, filled, 4);
    }
}

impl Model {
    /// A scenario aimed at the cancel path: an order part-fills, the venue cancels it (reporting
    /// nothing, as Bullet does), the ladder places the level again whole, and only then does the
    /// part fill's trade arrive. The order resting now would overfill the level, so the ladder must
    /// cancel it and keep asking until the venue answers.
    pub(super) fn inject_overfill(&mut self) {
        let fresh: Vec<String> = self
            .orders
            .iter()
            .filter(|(_, o)| o.venue_live && o.true_filled.is_zero() && !o.told_gone)
            .map(|(i, _)| i.clone())
            .collect();
        if fresh.is_empty() {
            return;
        }
        let id = fresh[usize::try_from(self.rng.below(fresh.len() as u64)).unwrap_or(0)].clone();
        let Some(o) = self.orders.get_mut(&id) else { return };
        let q = d("0.004").min(o.order.quantity);
        o.true_filled = q;
        let order = o.order.clone();
        let trade = Trade {
            exchange: "sim".into(),
            symbol: order.symbol.clone(),
            order_id: id.clone(),
            trade_id: None,
            client_id: order.client_id.clone(),
            side: order.side,
            price: order.price,
            quantity: q,
            timestamp: None,
        };
        self.queue.push(Queued {
            due: self.step + 6,
            event: on_bullet(SYM, MarketEvent::Fill(trade)),
            told: Told::Trade(id.clone(), q),
        });
        let was = self.chaos;
        self.chaos = false; // the cancel event arrives at once, reporting 0
        self.end_order(&id, OrderStatus::Cancelled, Decimal::ZERO, 0);
        // Another level's order is cancelled with it: together they are a sweep, which never
        // counts toward the cap that would hold the level.
        let other = self.live_ids().into_iter().find(|i| {
            self.orders.get(i).is_some_and(|o| {
                o.level != order.client_id.as_deref().map(level_of).unwrap_or_default()
            })
        });
        if let Some(other) = other {
            self.end_order(&other, OrderStatus::Cancelled, Decimal::ZERO, 0);
        }
        self.chaos = was;
        self.deliver_due();
        self.market.now_ms += 3_000;
        self.book_event(5);
    }
}

impl Model {
    /// A tick with a fresh venue read: it lists what rests at the venue now.
    pub(super) fn tick_with_read(&mut self) {
        let started = self.market.now_ms;
        let open: Vec<Order> = self
            .orders
            .iter()
            .filter(|(_, o)| o.venue_live)
            .map(|(id, o)| Order {
                id: format!("v-{id}"),
                client_id: Some(id.clone()),
                symbol: o.order.symbol.clone(),
                side: o.order.side,
                order_type: o.order.order_type,
                price: o.order.price,
                quantity: o.order.quantity,
                filled_quantity: o.true_filled,
                status: OrderStatus::Open,
            })
            .collect();
        let listed = open.iter().filter_map(|o| o.client_id.clone()).collect();
        self.read(Some(open), Some(started));
        self.deliver(&Event::Tick, &Told::Read(started, listed));
        self.read(Some(Vec::new()), None);
    }

    /// What Bullet's next view lists, and when that read started.
    fn read(&mut self, open: Option<Vec<Order>>, started: Option<u64>) {
        if let Some(bullet) = self.market.bullet_mut() {
            bullet.open_orders = open;
            bullet.read_started_ms = started;
        }
    }
}
