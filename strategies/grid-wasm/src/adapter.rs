//! The grid as a [`StrategyAdapter`]: bullet-bots' `grid` (`adapter.rs`) on the SDK. Data in,
//! intents out.
//!
//! Start: resolve the [`Plan`] from the settings' range and levels, refuse a worst case past the
//! dollars or the grid's own cap, lay out the levels (the one nearest the mid empty), open the
//! starting position with one IOC (long, short) and place every other level resting (post-only; a
//! level that would cross goes at the touch).
//!
//! Events:
//!   - `Fill` — the host has already recorded it in the inventory. A completed level goes dormant
//!     and re-arms its neighbour with the opposite side (a buy fills: a sell one level up).
//!   - `Order` — learn exchange `order_ids`; mark cancel/reject levels Pending so the tick
//!     re-places them.
//!   - `Outcome` — the answer to a `Place`: the level goes Active, or stays Pending for the next
//!     pass. A refused starting IOC stops the bot. A cancel-all that failed is asked again on every
//!     tick until one lands ([`Pull`]).
//!   - `Unknown` — a `Place` the venue left unanswered holds its level as placed; a read at least
//!     the settle margin later that does not list it frees the level.
//!   - `Book` — the hard stop is checked on every book.
//!   - `Tick` — the stop, the out-of-range pause, the trend filter, the missing-order reconcile,
//!     the pending placements; once stopped, the close is retried until flat.
//!
//! Safety (as the built-in; a failing test first):
//!   1. The worst case (every risk-side level filled) must fit the dollars and the grid's own cap
//!      ([`crate::plan`]).
//!   2. Past the hard stop (`stop_beyond` range widths past the losing edge): cancel its orders,
//!      close with reduce-only IOCs (the bot sizes them to the venue position) and stop.
//!   3. Out of range on the profitable side (long above the range, short below it): the opening
//!      orders are cancelled until the price is back; on the losing side the grid holds.
//!   4. The trend filter cancels the opening orders only.
//!   5. The report shows the host's realized and unrealized P&L of the whole position.
//!
//! What the built-in does that the `bb:strategy@0.4.0` world cannot carry: the
//! range and levels derived from past candles and the trend filter's warm-up on them (no candle
//! read); the hard limits the build hands it (`with_limits`: the view carries no limits); the
//! daily loss stop's probe (`with_day_stop`: the bot's own stop still halts its orders); the
//! cancel-all-failing alert (`Intent::Notify`); `CancelOwn` (a plugin's `CancelAll` takes only
//! its own orders, so it is the same); and its log lines.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal USD notionals, prices, sizes and bps sit far below 7.9e28; skip/placement \
              counters and close attempts are tiny"
)]

use std::collections::{HashMap, HashSet};

use bb_strategy_sdk::{
    CancelOrder, Decimal, Event, Intent, NewOrder, Order, OrderBook, OrderResult, OrderStatus,
    OrderType, Side, StrategyAdapter, StrategyError, Trade, View,
};
use bb_strategy_support::ioc::{capped_ioc_price, two_sided_mid};
use bb_strategy_support::{BookExt, ClientIdIssuer, side_word};

use crate::config::GridConfig;
use crate::grid::{FillOutcome, GridState, LevelState};
use crate::plan::{self, HardLimits, Market, Plan};
use crate::pull::Pull;
use crate::report::{GridPnl, GridReport};

mod trend;

/// How long after an unknown place a read must start before it is taken as showing the order (the
/// venue's REST can lag): the order path's settle margin.
const UNKNOWN_SETTLE_MS: u64 = 2_000;

/// How far past the touch the starting IOC may fill (bps).
pub const ENTRY_SLIP_BPS: u32 = 20;
/// How much further past the touch each close attempt may fill (bps).
pub const CLOSE_SLIP_BPS: u32 = 20;
/// The farthest any IOC trades from the mid (bps), the cap rsi-momentum uses too.
pub const MAX_IOC_BPS: u32 = 200;

/// Why the grid stopped trading, and how many closes it sent since.
#[derive(Debug, Clone)]
struct Halt {
    reason: String,
    attempts: u32,
    /// The cancel the stop asks, and asks again while it has not landed: the plugin's own orders.
    pull: Intent,
}

/// The starting position's IOC.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    None,
    Sent(String),
    Done,
}

/// What the last view showed: the report's P&L.
#[derive(Debug, Clone, Default)]
struct Seen {
    mid: Option<Decimal>,
    position: Decimal,
    avg_entry: Option<Decimal>,
    realized: Decimal,
}

#[derive(Debug)]
pub struct GridStrategy {
    config: GridConfig,
    /// The market traded: the view's, from the start.
    symbol: String,
    /// None: a plugin is handed no hard limits (the bot's gate still holds them).
    limits: HardLimits,
    plan: Option<Plan>,
    tick: Option<Decimal>,
    state: GridState,
    client_ids: ClientIdIssuer,
    entry: Entry,
    range_paused: bool,
    halt: Option<Halt>,
    seen: Seen,
    /// The last cancel-all, and whether it failed (see [`Pull`]).
    pull: Pull,
    /// Levels held for a place whose outcome is unknown: the client id and the bot clock (ms) of
    /// the event. A read frees one only if it is at least [`UNKNOWN_SETTLE_MS`] later.
    unknown_levels: HashMap<String, u64>,
}

fn bps(n: u32) -> Decimal {
    Decimal::from(n)
}

impl GridStrategy {
    pub fn new(config: GridConfig) -> Self {
        let state = match &config.trend_filter {
            Some(cfg) => GridState::new().with_trend_filter(cfg),
            None => GridState::new(),
        };
        Self {
            symbol: config.symbol.clone().unwrap_or_default(),
            config,
            limits: HardLimits::default(),
            plan: None,
            tick: None,
            state,
            client_ids: ClientIdIssuer::default(),
            entry: Entry::None,
            range_paused: false,
            halt: None,
            seen: Seen::default(),
            pull: Pull::default(),
            unknown_levels: HashMap::new(),
        }
    }

    fn symbol(&self) -> &str {
        &self.symbol
    }

    /// Refuse to start if the per-level spacing can't cover round-trip
    /// maker fees by the configured multiplier.
    fn check_fee_floor(&self, plan: &Plan) -> Result<(), StrategyError> {
        let Some(fees) = &self.config.fees else {
            return Ok(());
        };
        // Smallest as a fraction of price at the top of the range: the floor holds everywhere.
        let spacing = plan.spacing();
        if spacing.is_zero() || plan.upper.is_zero() {
            return Err(StrategyError("degenerate grid geometry".into()));
        }
        let spacing_bps = spacing / plan.upper * Decimal::from(10_000);
        let required_bps = Decimal::from(2) * fees.maker_bps * fees.min_spacing_fee_multiplier;
        if spacing_bps < required_bps {
            return Err(StrategyError(format!(
                "Grid spacing {spacing_bps} bps < required {required_bps} bps \
                 ({} × round-trip maker fee). Widen the range or reduce num_levels.",
                fees.min_spacing_fee_multiplier
            )));
        }
        Ok(())
    }

    fn order_size(&self) -> Decimal {
        self.plan.as_ref().map_or(Decimal::ZERO, |p| p.order_size)
    }

    /// The grid's own cap: the config's, else the worst case it was sized to.
    fn cap(&self) -> Decimal {
        let worst = self.plan.as_ref().map_or(Decimal::ZERO, |p| p.worst_coins);
        self.config.max_position.unwrap_or(worst)
    }

    fn paused(&self) -> bool {
        self.state.paused || self.range_paused
    }

    fn cancel_of(&self, i: usize) -> Option<Intent> {
        let l = &self.state.levels[i];
        if l.state != LevelState::Active {
            return None;
        }
        Some(Intent::Cancel(CancelOrder {
            symbol: self.symbol().to_string(),
            order_id: l.order_id.clone().unwrap_or_default(),
            client_id: l.client_id.clone(),
        }))
    }

    /// Hold every opening order back (closing ones stay): cancels for the resting ones.
    fn pause_opening(&mut self, position: Decimal) -> Vec<Intent> {
        let closing = self.state.closing(position, self.order_size());
        let mut out = Vec::new();
        for (i, closes) in closing.into_iter().enumerate() {
            let l = &self.state.levels[i];
            if closes || !matches!(l.state, LevelState::Active | LevelState::Pending) {
                continue;
            }
            out.extend(self.cancel_of(i));
            self.state.pause(i);
        }
        out
    }

    /// A `Place` for every `Pending` level, skipping sides at the cap; a post-only level that
    /// would cross goes at the touch. While paused, an opening level is paused instead. Each
    /// level keeps the client id its order carries, so the venue's answer finds it again.
    fn place_pending_orders(&mut self, view: &View<'_>) -> Vec<Intent> {
        if self.halt.is_some() || self.plan.is_none() {
            return Vec::new();
        }
        let order_type = self.config.order_type;
        let order_size = self.order_size();
        let max_position = self.cap();
        let net_position = view.position;
        let closing = self.state.closing(net_position, order_size);
        let mut projected_long = net_position + self.state.active_remaining(Side::Buy, order_size);
        let mut projected_short =
            net_position - self.state.active_remaining(Side::Sell, order_size);

        let mut orders: Vec<NewOrder> = Vec::new();

        for (idx, closes) in closing.into_iter().enumerate() {
            let (side, level_price) = {
                let l = &self.state.levels[idx];
                if l.state != LevelState::Pending {
                    continue;
                }
                let Some(side) = l.side else { continue };
                (side, l.price)
            };
            if self.paused() && !closes {
                self.state.pause(idx);
                continue;
            }
            let mut price = level_price;
            if order_type == OrderType::PostOnly
                && let Some(book) = view.book
                && book.would_cross(side, price)
            {
                // Repriced to the touch on its own side: still a maker order.
                let touch = match side {
                    Side::Buy => book.best_bid(),
                    Side::Sell => book.best_ask(),
                };
                match touch {
                    Some(t) if !book.would_cross(side, t.price) => price = t.price,
                    _ => continue,
                }
            }

            match side {
                Side::Buy if projected_long + order_size > max_position => {
                    continue;
                }
                Side::Buy => projected_long += order_size,
                Side::Sell if projected_short - order_size < -max_position => {
                    continue;
                }
                Side::Sell => projected_short -= order_size,
            }

            let cid = self.client_ids.issue();
            self.state.levels[idx].client_id = Some(cid.clone());
            orders.push(NewOrder {
                symbol: self.symbol().to_string(),
                side,
                order_type,
                price,
                quantity: order_size,
                client_id: Some(cid),
                reduce_only: false,
                protect: None,
            });
        }

        orders.into_iter().map(Intent::Place).collect()
    }

    /// The venue's answer to a `Place`: the level goes Active, or back to retry.
    fn placed(&mut self, order: &NewOrder, result: &OrderResult) -> Vec<Intent> {
        if let Entry::Sent(cid) = &self.entry
            && order.client_id.as_deref() == Some(cid.as_str())
        {
            if result.success {
                self.entry = Entry::Done;
                return Vec::new();
            }
            let why = result.error.as_deref().unwrap_or("refused");
            let reason = format!(
                "the starting {} of {} {} didn't go through ({why}); it stopped rather than run \
                 without it",
                side_word(order.side),
                order.quantity,
                self.symbol()
            );
            // The built-in asks the bot to stop here (`Intent::Shutdown`). A bot refuses that from a
            // plugin (and faults it), so the grid stops itself: its orders go, it places nothing
            // more, and a tick closes any position the IOC left.
            self.state.clear_all();
            self.halt = Some(Halt { reason, attempts: 1, pull: Intent::CancelAll });
            return vec![Intent::CancelAll];
        }
        let cid = order.client_id.as_deref();
        let Some(level) =
            self.state.levels.iter_mut().find(|l| cid.is_some() && l.client_id.as_deref() == cid)
        else {
            return Vec::new();
        };
        if result.success {
            level.state = LevelState::Active;
            if let Some(ref oid) = result.order_id {
                level.order_id = Some(oid.clone());
            }
        } else {
            // The host logs each refusal once (rate-limited): not again here, once per retry.
            level.client_id = None;
        }
        Vec::new()
    }

    /// Missing-order reconcile: any `Active` level whose order is not among the
    /// venue's live orders (e.g., silently cancelled by a venue edge case)
    /// flips back to `Pending` so we re-place next tick.
    fn reconcile_missing_orders(&mut self, live: &[Order], read_ms: u64) -> usize {
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
            let held = l.client_id.as_ref().and_then(|c| self.unknown_levels.get(c)).copied();
            if is_live {
                // It turned out to rest: no longer unknown.
                if let Some(c) = &l.client_id {
                    self.unknown_levels.remove(c);
                }
            }
            if !is_live && held.is_some_and(|at| read_ms < at.saturating_add(UNKNOWN_SETTLE_MS)) {
                // A read just after the unknown place may not list it yet (REST lag): held.
                continue;
            }
            if !is_live {
                if let Some(c) = &l.client_id {
                    self.unknown_levels.remove(c);
                }
                l.state = LevelState::Pending;
                l.client_id = None;
                l.order_id = None;
                missing += 1;
            }
        }
        missing
    }

    /// The reason to stop now, if any, and the cancel it asks: the hard stop past the range, which
    /// ends the grid's own trading, so only its own orders go (a plugin's [`Intent::CancelAll`]
    /// takes only its own; the built-in asks `CancelOwn`).
    fn stop_reason(&self, mid: Option<Decimal>) -> Option<(String, Intent)> {
        let plan = self.plan.as_ref()?;
        if let Some(mid) = mid {
            if let Some(stop) = plan.stop_below
                && mid <= stop
            {
                let why = format!("hard stop: {} at {mid}, at or below {stop}", self.symbol());
                return Some((why, Intent::CancelAll));
            }
            if let Some(stop) = plan.stop_above
                && mid >= stop
            {
                let why = format!("hard stop: {} at {mid}, at or above {stop}", self.symbol());
                return Some((why, Intent::CancelAll));
            }
        }
        // The built-in also stops here when the build's daily loss probe trips; a plugin has none
        // (the bot's own daily loss stop halts its orders).
        None
    }

    /// A reduce-only IOC closing `view.position`, capped around the mid; none when flat or with
    /// no two-sided book. The order path holds it to the venue's position.
    fn close(&mut self, view: &View<'_>, attempt: u32) -> Option<Intent> {
        let position = view.position;
        if position.is_zero() {
            return None;
        }
        let book = view.book?;
        let mid = two_sided_mid(book)?;
        let side = if position > Decimal::ZERO { Side::Sell } else { Side::Buy };
        let touch = match side {
            Side::Sell => book.best_bid()?.price,
            Side::Buy => book.best_ask()?.price,
        };
        let slip = bps(CLOSE_SLIP_BPS) * Decimal::from(attempt.max(1));
        let price = capped_ioc_price(side, touch, mid, slip, bps(MAX_IOC_BPS))?;
        Some(Intent::Place(NewOrder {
            symbol: self.symbol().to_string(),
            side,
            order_type: OrderType::Market,
            price,
            quantity: position.abs(),
            client_id: Some(self.client_ids.issue()),
            reduce_only: true,
            protect: None,
        }))
    }

    /// Rule 2 and the daily loss stop: cancel, close, stop placing. Once stopped, a tick
    /// closes again while the position is open.
    fn check_halt(&mut self, view: &View<'_>, tick: bool) -> Option<Vec<Intent>> {
        if let Some(halt) = &mut self.halt {
            if !tick || view.position.is_zero() {
                return Some(Vec::new());
            }
            halt.attempts = halt.attempts.saturating_add(1);
            let attempt = halt.attempts;
            return Some(self.close(view, attempt).into_iter().collect());
        }
        let mid = view.book.and_then(OrderBook::midpoint);
        let (reason, pull) = self.stop_reason(mid)?;
        self.state.clear_all();
        self.halt = Some(Halt { reason, attempts: 1, pull: pull.clone() });
        let mut out = vec![pull];
        out.extend(self.close(view, 1));
        Some(out)
    }

    fn see(&mut self, view: &View<'_>) {
        self.seen = Seen {
            mid: view.book.and_then(OrderBook::midpoint).or(self.seen.mid),
            position: view.position,
            avg_entry: view.avg_entry,
            realized: view.realized_pnl,
        };
    }

    fn on_fill(&mut self, event: &Trade, view: &View<'_>) -> Vec<Intent> {
        if let Entry::Sent(cid) = &self.entry
            && event.client_id.as_deref() == Some(cid.as_str())
        {
            return Vec::new();
        }
        let Some(idx) = self.state.find_fill_target(event.client_id.as_deref(), &event.order_id)
        else {
            return Vec::new();
        };
        // A completed level re-arms its neighbour; the placement below sends it.
        let _: FillOutcome = self.state.record_fill(idx, event.quantity, self.order_size());
        self.place_pending_orders(view)
    }

    fn on_order(&mut self, order: &Order) {
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
            level.state = LevelState::Pending;
            level.client_id = None;
            level.order_id = None;
        }
    }

    fn on_book(&mut self, view: &View<'_>) -> Vec<Intent> {
        self.check_halt(view, false).unwrap_or_default()
    }

    /// While the stop's cancel has not landed, orders may still rest: it is asked again, the same
    /// kind (a cancel-all only a stop asks for, so a failed one has a stop).
    fn ask_pull_again(&self, intents: &mut Vec<Intent>) {
        let pulls = |i: &Intent| matches!(i, Intent::CancelAll);
        if self.pull.failed() && !intents.iter().any(pulls) {
            let pull = self.halt.as_ref().map_or(Intent::CancelAll, |h| h.pull.clone());
            intents.push(pull);
        }
    }

    fn on_tick(&mut self, view: &View<'_>) -> Vec<Intent> {
        if let Some(mut halted) = self.check_halt(view, true) {
            self.ask_pull_again(&mut halted);
            return halted;
        }
        let mut intents = Vec::new();
        if let Some(mid) = view.book.and_then(OrderBook::midpoint) {
            intents.extend(self.handle_range(mid, view.position));
        }
        intents.extend(self.handle_trend_filter(view));
        // Nothing is placed while a cancel-all has not landed.
        if self.pull.failed() {
            self.ask_pull_again(&mut intents);
            return intents;
        }
        // With no venue read this tick, the levels keep their state: a stale list would show
        // every level as gone and the grid would place it all again.
        if let Some(live) = view.open_orders {
            // The built-in logs how many it found missing; they go back to pending either way.
            let _missing =
                self.reconcile_missing_orders(live, view.read_started_ms.unwrap_or(view.now_ms));
        }
        intents.extend(self.place_pending_orders(view));
        intents
    }

    /// The starting position's IOC at the touch, capped around the mid.
    fn entry_order(
        &mut self,
        (side, qty): (Side, Decimal),
        book: &OrderBook,
    ) -> Result<NewOrder, StrategyError> {
        let no_book = || {
            StrategyError(format!(
                "No two-sided book on {} to open the starting position",
                self.symbol()
            ))
        };
        let mid = two_sided_mid(book).ok_or_else(no_book)?;
        let touch = match side {
            Side::Buy => book.best_ask(),
            Side::Sell => book.best_bid(),
        }
        .ok_or_else(no_book)?
        .price;
        let price = capped_ioc_price(side, touch, mid, bps(ENTRY_SLIP_BPS), bps(MAX_IOC_BPS))
            .ok_or_else(no_book)?;
        let cid = self.client_ids.issue();
        self.entry = Entry::Sent(cid.clone());
        Ok(NewOrder {
            symbol: self.symbol().to_string(),
            side,
            order_type: OrderType::Market,
            price,
            quantity: qty,
            client_id: Some(cid),
            reduce_only: false,
            protect: None,
        })
    }
}

impl StrategyAdapter for GridStrategy {
    type Report = GridReport;

    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        self.config.check()?;
        if let Some(symbol) = self.config.symbol.as_deref()
            && symbol != view.symbol
        {
            return Err(StrategyError(format!(
                "grid: the settings name {symbol}, the bot trades {}",
                view.symbol
            )));
        }
        view.symbol.clone_into(&mut self.symbol);
        self.config.symbol = Some(view.symbol.to_owned());
        // The bot's own ids, seeded from its clock (a component has no other).
        self.client_ids = ClientIdIssuer::own(view.now_ms);
        if !view.position.is_zero() {
            return Err(StrategyError(format!(
                "The account already holds {} {}: the grid starts from no position. Close it on \
                 the venue (or use another sub-account), then start the grid.",
                view.position,
                self.symbol()
            )));
        }
        let book = view.book.ok_or_else(|| {
            StrategyError("No orderbook data available to compute mid at startup".into())
        })?;
        let mid = book.midpoint().ok_or_else(|| {
            StrategyError("No orderbook data available to compute mid at startup".into())
        })?;
        self.tick = view.market.tick_size;
        let market = Market { mid, tick: view.market.tick_size, lot: view.market.step_size };
        let plan = plan::resolve(&self.config, market, None, self.limits).map_err(StrategyError)?;
        self.check_fee_floor(&plan)?;
        self.state.compute_levels(&plan, self.tick);
        let entry = plan.entry;
        self.plan = Some(plan);
        self.see(view);

        // The trend filter before the first placement: a market already trending at start holds
        // the opening orders back before they go out.
        let mut intents = self.handle_trend_filter(view);
        if let Some(entry) = entry {
            intents.push(Intent::Place(self.entry_order(entry, book)?));
        }
        intents.extend(self.place_pending_orders(view));
        Ok(intents)
    }

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent> {
        // A cancel-all's answer counts even before the plan: the stop's cancel-all may fail too.
        if let Event::Outcome { intent: Intent::CancelAll, result } = event {
            let why = result.error.as_deref().unwrap_or("refused");
            return self.pull.answered((!result.success).then_some(why)).into_iter().collect();
        }
        if self.plan.is_none() {
            return Vec::new();
        }
        self.see(view);
        match event {
            Event::Fill(trade) => self.on_fill(trade, view),
            Event::Order(order) => {
                self.on_order(order);
                Vec::new()
            }
            Event::Book => self.on_book(view),
            Event::Tick => self.on_tick(view),
            Event::Outcome { intent: Intent::Place(order), result } => self.placed(order, result),
            // A place that may have landed holds its level as placed (order id unknown): it is not
            // placed again. A later read that does not list it frees the level (reconcile).
            Event::Unknown { intent: Intent::Place(order), .. } => {
                let held = OrderResult {
                    order_id: None,
                    client_id: order.client_id.clone(),
                    success: true,
                    error: None,
                };
                if let Some(c) = &order.client_id {
                    self.unknown_levels.insert(c.clone(), view.now_ms);
                }
                self.placed(order, &held)
            }
            // Other outcomes, and any event a later bb-strategy adds: nothing to do.
            _ => Vec::new(),
        }
    }

    fn stop(&mut self, view: &View<'_>) -> Vec<Intent> {
        // A stopped grid with a position left closes it on the way out; otherwise cancel only (a
        // reconfigure's flatten is the supervisor's).
        let mut out = vec![Intent::CancelAll];
        if let Some(attempt) = self.halt.as_ref().map(|h| h.attempts.saturating_add(1)) {
            out.extend(self.close(view, attempt));
        }
        out
    }

    fn report(&self) -> GridReport {
        let Seen { mid, position, avg_entry, realized } = self.seen.clone();
        let unrealized = match (mid, avg_entry) {
            (Some(m), Some(e)) => (m - e) * position,
            _ => Decimal::ZERO,
        };
        let usd = |d: Decimal| d.round_dp(2).normalize();
        let plan = self.plan.as_ref();
        GridReport {
            direction: plan.map(|p| format!("{:?}", p.direction).to_lowercase()),
            range: format!("{:?}", self.config.range).to_lowercase(),
            lower_price: plan.map(|p| p.lower),
            upper_price: plan.map(|p| p.upper),
            anchor: plan.map(|p| p.anchor),
            spacing: plan.map(Plan::spacing),
            order_size: plan.map(|p| p.order_size),
            stop_below: plan.and_then(|p| p.stop_below),
            stop_above: plan.and_then(|p| p.stop_above),
            worst_case_usd: plan.map(|p| usd(p.worst_notional)),
            amount_usd: plan.map(|p| p.dollars),
            active_levels: self.state.active_count(),
            total_levels: self.state.levels.len(),
            paused: self.paused(),
            stopped: self.halt.as_ref().map(|h| h.reason.clone()),
            pnl: GridPnl {
                realized: usd(realized),
                unrealized: usd(unrealized),
                total: usd(realized + unrealized),
            },
            cancel_failures: self.pull.failures(),
            levels: self.state.levels.clone(),
        }
    }
}

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod tests;
