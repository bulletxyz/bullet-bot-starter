//! [`FundingArb`]: carries out [`decide`](crate::decide)'s target on a Bullet leg (the one the bot
//! trades) and a Hyperliquid leg (`hedge`). **New code of this plugin, not a port of the bot's
//! safety executor**, which a plugin cannot be: the `bb:strategy@0.5.0` world has no `Targets`
//! intent, so the plugin places its own orders.
//!
//! Every [`ROUND_SECS`] (sooner after an action) it reads both venues' positions and equity,
//! both funding histories and Bullet's open interest (requests, answered between events), builds a
//! one-symbol view, calls `decide`, and acts:
//!
//! - flat and `decide` wants a position: one IOC on Bullet, sized to the target; the next round
//!   sees what filled and sends the Hyperliquid IOC sized to that, opposite;
//! - holding a matched pair and `decide` wants it: nothing (it never grows or trims);
//! - holding a matched pair and `decide` wants 0: reduce-only IOCs close both;
//! - anything else (one leg only, same direction on both, sizes more than [`TOLERANCE`] apart, a
//!   hedge that could not be priced): it closes what is open, wider each round, until both read flat.
//!
//! What it does not do, against the built-in with its executor: no clamping to the executor's hard
//! caps (only `notional_per_symbol` on what it sends); no hedge repair other than closing (a
//! mismatch is unwound, never topped up); no reconcile loop against fills or open orders (positions
//! are re-read each round, and that is all); no rebalancing or transfers between venues; no
//! `Stuck` state or alert (a close that keeps failing shows in the report's `unwind_attempts`); one
//! symbol per instance, so `exclude`/`only`/`max_positions` do not apply; Bullet's positions
//! outside the symbol are valued at their entry price for the leverage budget. The bot's own gate
//! and daily loss stop still apply to every order. A flatten on stop is the bot's `Flatten`, which
//! it refuses on the symbol it trades when the account is the owner's main one.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal sizes, prices and bps sit far below 7.9e28; divisors are checked non-zero; counters saturate"
)]

use std::collections::{BTreeMap, BTreeSet};

use bb_strategy_sdk::{
    Answer, Decimal, Event, FundingSample, Intent, LegView, NewOrder, OrderBook, OrderType,
    Position, ReadQuery, Side, StrategyAdapter, StrategyError, TRADE_LEG, VenueCall, View,
    signed_position,
};
use bb_strategy_support::ioc::{capped_ioc_price, two_sided_mid, within_bps};
use bb_strategy_support::{ClientIdIssuer, close_reference, touch};

use crate::config::FundingArbSettings;
use crate::decide::{Direction, Leg, MarketView, SymbolView, decide};
use crate::report::ArbReport;
use crate::view::{round_trip_cost, signal_8h};

/// The leg the plugin trades the hedge on: `hyperliquid`, one symbol.
pub const HEDGE_LEG: &str = "hedge";
/// A full read round this often.
pub const ROUND_SECS: u64 = 300;
/// After an action, or a failed read: the next round comes after this.
pub const SOON_SECS: u64 = 5;
/// A book or mark older than this (ms) is not a price.
pub const STALE_MS: u64 = 30_000;
/// No IOC is priced further than this from the book's mid (bps).
pub const MAX_IOC_BPS: Decimal = Decimal::from_parts(200, 0, 0, false, 0);
/// A pair's legs may differ by this share of the larger before it is unwound.
pub const TOLERANCE: Decimal = Decimal::from_parts(1, 0, 0, false, 1);
/// A first leg waits this long (ms) for its hedge to be sent.
pub const HEDGE_WITHIN_MS: u64 = 30_000;
/// A close crosses `slippage_bps` times this, times the attempt (at most [`MAX_ATTEMPT`]).
pub const CLOSE_SLIPPAGE_MULT: u32 = 3;
pub const MAX_ATTEMPT: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Read {
    PosBullet,
    PosHl,
    EqBullet,
    EqHl,
    FundBullet,
    FundHl,
    Oi,
}

#[derive(Debug, Default)]
struct Round {
    pending: BTreeMap<u64, Read>,
    pos_bullet: Option<Vec<Position>>,
    pos_hl: Option<Vec<Position>>,
    eq_bullet: Option<Decimal>,
    eq_hl: Option<Decimal>,
    fund_bullet: Vec<FundingSample>,
    fund_hl: Vec<FundingSample>,
    oi: Option<Decimal>,
    failed: Option<String>,
}

#[derive(Debug, Default)]
enum Flow {
    #[default]
    Idle,
    Reading(Round),
    /// Order requests in flight.
    Acting(BTreeSet<u64>),
}

/// A first leg sent, whose hedge has not been.
#[derive(Debug, Clone, Copy)]
struct Pending {
    direction: Direction,
    at_ms: u64,
    hedge_sent: bool,
}

/// The funding arb on a Bullet leg and a Hyperliquid leg.
#[derive(Debug)]
pub struct FundingArb {
    cfg: FundingArbSettings,
    flow: Flow,
    next_at_ms: u64,
    next_id: u64,
    ids: ClientIdIssuer,
    mark: Option<(Decimal, u64)>,
    book_ms: Option<u64>,
    pending: Option<Pending>,
    attempts: u32,
    report: ArbReport,
}

struct Hedge<'a> {
    leg: &'a LegView,
    symbol: &'a str,
}

fn hedge<'a>(view: &'a View<'_>) -> Option<Hedge<'a>> {
    let leg = view.legs.iter().find(|l| l.id == HEDGE_LEG)?;
    match leg.symbols.as_slice() {
        [symbol] => Some(Hedge { leg, symbol }),
        _ => None,
    }
}

fn floor_to_step(qty: Decimal, step: Decimal) -> Decimal {
    if step <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    (qty / step).floor() * step
}

fn side_of(signed: Decimal) -> Side {
    if signed > Decimal::ZERO { Side::Buy } else { Side::Sell }
}

impl FundingArb {
    #[must_use]
    pub fn new(cfg: FundingArbSettings) -> Self {
        Self {
            cfg,
            flow: Flow::Idle,
            next_at_ms: 0,
            next_id: 1,
            ids: ClientIdIssuer::starting_at(1),
            mark: None,
            book_ms: None,
            pending: None,
            attempts: 0,
            report: ArbReport { state: "idle".into(), ..ArbReport::default() },
        }
    }

    fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn note(&mut self, text: impl Into<String>) {
        self.report.note = Some(text.into());
    }

    fn fresh_mark(&self, now_ms: u64) -> Option<Decimal> {
        self.mark.filter(|(_, at)| now_ms.saturating_sub(*at) <= STALE_MS).map(|(m, _)| m)
    }

    fn fresh_book<'a>(&self, view: &'a View<'_>) -> Option<&'a OrderBook> {
        let age = view.now_ms.saturating_sub(self.book_ms?);
        view.book.filter(|_| age <= STALE_MS)
    }

    fn start_round(&mut self, view: &View<'_>, h: &Hedge<'_>) -> Vec<Intent> {
        let start_ms = i64::try_from(
            view.now_ms.saturating_sub(self.cfg.signal_window_hours.saturating_mul(3_600_000)),
        )
        .unwrap_or(0);
        let hedge_symbol = h.symbol.to_owned();
        let asks = [
            (TRADE_LEG, ReadQuery::Positions, Read::PosBullet),
            (HEDGE_LEG, ReadQuery::Positions, Read::PosHl),
            (TRADE_LEG, ReadQuery::AccountEquity, Read::EqBullet),
            (HEDGE_LEG, ReadQuery::AccountEquity, Read::EqHl),
            (
                TRADE_LEG,
                ReadQuery::FundingHistory { symbol: view.symbol.to_owned(), start_ms },
                Read::FundBullet,
            ),
            (HEDGE_LEG, ReadQuery::FundingHistory { symbol: hedge_symbol, start_ms }, Read::FundHl),
            (TRADE_LEG, ReadQuery::OpenInterest { symbol: view.symbol.to_owned() }, Read::Oi),
        ];
        let mut round = Round::default();
        let mut intents = Vec::new();
        for (leg, query, kind) in asks {
            let id = self.id();
            round.pending.insert(id, kind);
            intents.push(Intent::Request { id, leg: leg.into(), call: VenueCall::Read(query) });
        }
        self.flow = Flow::Reading(round);
        "reading".clone_into(&mut self.report.state);
        intents
    }

    fn on_answer(
        &mut self,
        id: u64,
        reply: &Result<Answer, bb_strategy_sdk::CallError>,
        view: &View<'_>,
    ) -> Vec<Intent> {
        match &mut self.flow {
            Flow::Reading(round) => {
                let Some(kind) = round.pending.remove(&id) else { return Vec::new() };
                match (reply, kind) {
                    (Ok(Answer::Positions(p)), Read::PosBullet) => {
                        round.pos_bullet = Some(p.clone());
                    }
                    (Ok(Answer::Positions(p)), Read::PosHl) => round.pos_hl = Some(p.clone()),
                    (Ok(Answer::AccountEquity(e)), Read::EqBullet) => {
                        round.eq_bullet = e.as_ref().map(|e| e.equity);
                    }
                    (Ok(Answer::AccountEquity(e)), Read::EqHl) => {
                        round.eq_hl = e.as_ref().map(|e| e.equity);
                    }
                    (Ok(Answer::FundingHistory(f)), Read::FundBullet) => {
                        round.fund_bullet.clone_from(f);
                    }
                    (Ok(Answer::FundingHistory(f)), Read::FundHl) => round.fund_hl.clone_from(f),
                    (Ok(Answer::OpenInterest(o)), Read::Oi) => round.oi = *o,
                    (Err(e), _) => round.failed = Some(e.message.clone()),
                    _ => round.failed = Some(format!("an answer of the wrong kind for {kind:?}")),
                }
                if !round.pending.is_empty() {
                    return Vec::new();
                }
                let Flow::Reading(round) = std::mem::take(&mut self.flow) else {
                    return Vec::new();
                };
                self.finish_round(&round, view)
            }
            Flow::Acting(ids) => {
                if ids.remove(&id) {
                    match reply {
                        Ok(Answer::Placed(rs)) => {
                            if let Some(why) =
                                rs.iter().find(|r| !r.success).and_then(|r| r.error.clone())
                            {
                                self.report.note = Some(format!("an order was refused: {why}"));
                            }
                        }
                        Ok(_) => {}
                        Err(e) => {
                            self.report.note = Some(format!("an order call failed: {}", e.message));
                        }
                    }
                }
                if let Flow::Acting(ids) = &self.flow
                    && ids.is_empty()
                {
                    self.flow = Flow::Idle;
                    "idle".clone_into(&mut self.report.state);
                    self.next_at_ms = view.now_ms.saturating_add(SOON_SECS * 1_000);
                }
                Vec::new()
            }
            Flow::Idle => Vec::new(),
        }
    }

    /// All reads are in: decide and act. A failed or missing read trades nothing.
    fn finish_round(&mut self, round: &Round, view: &View<'_>) -> Vec<Intent> {
        self.flow = Flow::Idle;
        "idle".clone_into(&mut self.report.state);
        self.next_at_ms = view.now_ms.saturating_add(SOON_SECS * 1_000);
        if let Some(why) = &round.failed {
            self.note(format!("a read failed, nothing traded: {why}"));
            return Vec::new();
        }
        let (Some(pos_b), Some(pos_h), Some(h)) =
            (round.pos_bullet.as_deref(), round.pos_hl.as_deref(), hedge(view))
        else {
            self.note("positions unread, nothing traded");
            return Vec::new();
        };
        let (b_base, h_base) =
            (signed_position(pos_b, view.symbol), signed_position(pos_h, h.symbol));
        let target = self.decide_target(round, view, &h, (b_base, h_base));
        self.report.bullet_base = Some(b_base);
        self.report.hl_base = Some(h_base);
        let wants = target.as_ref().map_or(Decimal::ZERO, |t| t.1);
        let intents =
            self.act(view, &h, pos_b, pos_h, (b_base, h_base), target.map(|t| (t.0, wants)));
        if intents.is_empty() {
            self.next_at_ms = view.now_ms.saturating_add(ROUND_SECS * 1_000);
        } else {
            let ids: BTreeSet<u64> = intents
                .iter()
                .filter_map(|i| if let Intent::Request { id, .. } = i { Some(*id) } else { None })
                .collect();
            self.flow = Flow::Acting(ids);
            "acting".clone_into(&mut self.report.state);
        }
        intents
    }

    /// `decide` on a one-symbol view: the direction and notional it wants.
    fn decide_target(
        &mut self,
        round: &Round,
        view: &View<'_>,
        h: &Hedge<'_>,
        (b_base, h_base): (Decimal, Decimal),
    ) -> Option<(Direction, Decimal)> {
        let mark = self.fresh_mark(view.now_ms).unwrap_or(Decimal::ZERO);
        let book_b = self.fresh_book(view);
        let book_h = h
            .leg
            .book(h.symbol)
            .filter(|_| h.leg.book_age_ms(h.symbol, view.now_ms).is_some_and(|a| a <= STALE_MS));
        let cost = match (book_b, book_h) {
            (Some(b), Some(hb)) => round_trip_cost(
                view.market.taker_fee_bps,
                h.leg.market.get(h.symbol).and_then(|m| m.taker_fee_bps),
                b,
                hb,
            ),
            _ => None,
        };
        let signal = signal_8h(&round.fund_bullet, &round.fund_hl);
        let others = |ps: &[Position], symbol: &str| -> Decimal {
            ps.iter().filter(|p| p.symbol != symbol).map(|p| p.size.abs() * p.entry_price).sum()
        };
        let params = self.cfg.params();
        let market = MarketView {
            symbols: vec![SymbolView {
                symbol: view.symbol.to_owned(),
                signal_8h: signal,
                round_trip_cost: cost,
                mark,
                bullet_oi_usd: round.oi.filter(|_| mark > Decimal::ZERO).map(|oi| oi * mark),
                leg_bullet: Leg { base: b_base },
                leg_hl: Leg { base: h_base },
            }],
            equity_bullet: round.eq_bullet,
            equity_hl: round.eq_hl,
            foreign_notional_bullet: round
                .pos_bullet
                .as_deref()
                .map_or(Decimal::ZERO, |p| others(p, view.symbol)),
            foreign_notional_hl: round
                .pos_hl
                .as_deref()
                .map_or(Decimal::ZERO, |p| others(p, h.symbol)),
            params,
        };
        let target = decide(&market).into_iter().next()?;
        // The one clamp this plugin keeps: never send more than the setting.
        let usd = target.notional_usd.min(self.cfg.notional_per_symbol);
        self.report.signal_8h = signal;
        self.report.round_trip_cost = cost;
        self.report.target_usd = Some(usd);
        self.report.direction = Some(format!("{:?}", target.direction));
        Some((target.direction, usd))
    }

    fn act(
        &mut self,
        view: &View<'_>,
        h: &Hedge<'_>,
        pos_b: &[Position],
        pos_h: &[Position],
        (b_base, h_base): (Decimal, Decimal),
        target: Option<(Direction, Decimal)>,
    ) -> Vec<Intent> {
        let (b_open, h_open) = (!b_base.is_zero(), !h_base.is_zero());
        let wants = target.is_some_and(|t| t.1 > Decimal::ZERO);
        match (b_open, h_open) {
            (false, false) => {
                self.pending = None;
                self.attempts = 0;
                self.report.unwind_attempts = 0;
                match target {
                    Some((direction, usd)) if wants => self.enter(view, h, direction, usd),
                    _ => Vec::new(),
                }
            }
            (true, false) => {
                if let Some(p) = self.pending.filter(|p| !p.hedge_sent) {
                    let fits = (p.direction == Direction::ShortBullet) == (b_base < Decimal::ZERO);
                    if fits && view.now_ms.saturating_sub(p.at_ms) <= HEDGE_WITHIN_MS {
                        let sent = self.send_hedge(view, h, b_base);
                        if !sent.is_empty() {
                            self.pending = Some(Pending { hedge_sent: true, ..p });
                            return sent;
                        }
                    }
                }
                self.pending = None;
                self.note("one-sided: closing the Bullet leg");
                self.unwind(view, h, pos_b, pos_h, (b_base, h_base))
            }
            (false, true) => {
                self.pending = None;
                self.note("one-sided: closing the hedge leg");
                self.unwind(view, h, pos_b, pos_h, (b_base, h_base))
            }
            (true, true) => {
                self.pending = None;
                let larger = b_base.abs().max(h_base.abs());
                let mismatched = (b_base < Decimal::ZERO) == (h_base < Decimal::ZERO)
                    || (b_base.abs() - h_base.abs()).abs() > larger * TOLERANCE;
                if mismatched {
                    self.note("the legs do not match: closing both");
                    return self.unwind(view, h, pos_b, pos_h, (b_base, h_base));
                }
                if wants {
                    self.attempts = 0;
                    self.report.unwind_attempts = 0;
                    return Vec::new();
                }
                self.note("exit: closing both legs");
                self.unwind(view, h, pos_b, pos_h, (b_base, h_base))
            }
        }
    }

    /// The first leg of a pair: one IOC on Bullet. Every gate says no, never guesses.
    fn enter(
        &mut self,
        view: &View<'_>,
        h: &Hedge<'_>,
        direction: Direction,
        usd: Decimal,
    ) -> Vec<Intent> {
        if let Some(why) = view.path_stop.or(view.venue_halt) {
            self.note(format!("entry blocked: {why}"));
            return Vec::new();
        }
        let (Some(mark), Some(book_b)) = (self.fresh_mark(view.now_ms), self.fresh_book(view))
        else {
            self.note("entry blocked: no fresh Bullet mark and book");
            return Vec::new();
        };
        let book_h = h
            .leg
            .book(h.symbol)
            .filter(|_| h.leg.book_age_ms(h.symbol, view.now_ms).is_some_and(|a| a <= STALE_MS));
        let (Some(mid_b), Some(_)) = (
            two_sided_mid(book_b).filter(|m| within_bps(*m, mark, MAX_IOC_BPS)),
            book_h.and_then(two_sided_mid),
        ) else {
            self.note("entry blocked: a book is stale, one-sided or far from the mark");
            return Vec::new();
        };
        let steps = (
            view.market.step_size.filter(|s| *s > Decimal::ZERO),
            h.leg.market.get(h.symbol).and_then(|m| m.step_size).filter(|s| *s > Decimal::ZERO),
        );
        let (Some(step_b), Some(step_h)) = steps else {
            self.note("entry blocked: a size step is unknown");
            return Vec::new();
        };
        let (coarse, fine) = (step_b.max(step_h), step_b.min(step_h));
        if !(coarse % fine).is_zero() {
            self.note("entry blocked: the venues' size steps do not divide");
            return Vec::new();
        }
        let qty = floor_to_step(usd / mark, coarse);
        if qty <= Decimal::ZERO {
            return Vec::new();
        }
        let side = if direction == Direction::ShortBullet { Side::Sell } else { Side::Buy };
        let base = touch(book_b, side).unwrap_or(mid_b);
        let Some(price) = capped_ioc_price(side, base, mid_b, self.cfg.slippage_bps, MAX_IOC_BPS)
        else {
            return Vec::new();
        };
        self.pending = Some(Pending { direction, at_ms: view.now_ms, hedge_sent: false });
        vec![self.place(TRADE_LEG, view.symbol, side, price, qty, false)]
    }

    /// The hedge leg, sized to what the first leg filled (floored to the hedge's step).
    fn send_hedge(&mut self, view: &View<'_>, h: &Hedge<'_>, b_base: Decimal) -> Vec<Intent> {
        let Some(step) = h.leg.market.get(h.symbol).and_then(|m| m.step_size) else {
            return Vec::new();
        };
        let qty = floor_to_step(b_base.abs(), step);
        let book = h
            .leg
            .book(h.symbol)
            .filter(|_| h.leg.book_age_ms(h.symbol, view.now_ms).is_some_and(|a| a <= STALE_MS));
        let Some((book, mid)) = book.and_then(|b| Some((b, two_sided_mid(b)?))) else {
            return Vec::new();
        };
        let side = if b_base < Decimal::ZERO { Side::Buy } else { Side::Sell };
        let base = touch(book, side).unwrap_or(mid);
        match capped_ioc_price(side, base, mid, self.cfg.slippage_bps, MAX_IOC_BPS) {
            Some(price) if qty > Decimal::ZERO => {
                vec![self.place(HEDGE_LEG, h.symbol, side, price, qty, false)]
            }
            _ => Vec::new(),
        }
    }

    /// Reduce-only closes of every open leg, crossing wider each consecutive round.
    fn unwind(
        &mut self,
        view: &View<'_>,
        h: &Hedge<'_>,
        pos_b: &[Position],
        pos_h: &[Position],
        (b_base, h_base): (Decimal, Decimal),
    ) -> Vec<Intent> {
        self.attempts = self.attempts.saturating_add(1);
        self.report.unwind_attempts = self.attempts;
        let mult =
            Decimal::from(CLOSE_SLIPPAGE_MULT.saturating_mul(self.attempts.min(MAX_ATTEMPT)));
        let slip = self.cfg.slippage_bps * mult;
        let mut out = Vec::new();
        if !b_base.is_zero() {
            let side = side_of(-b_base);
            let entry = pos_b.iter().find(|p| p.symbol == view.symbol).map(|p| p.entry_price);
            let age = self.book_ms.map(|at| view.now_ms.saturating_sub(at));
            if let Some(price) =
                Self::close_price(view.book, age, side, self.fresh_mark(view.now_ms), entry, slip)
            {
                out.push(self.place(TRADE_LEG, view.symbol, side, price, b_base.abs(), true));
            }
        }
        if !h_base.is_zero() {
            let side = side_of(-h_base);
            let entry = pos_h.iter().find(|p| p.symbol == h.symbol).map(|p| p.entry_price);
            let age = h.leg.book_age_ms(h.symbol, view.now_ms);
            let mark = h.leg.marks.get(h.symbol).copied();
            if let Some(price) =
                Self::close_price(h.leg.book(h.symbol), age, side, mark, entry, slip)
            {
                out.push(self.place(HEDGE_LEG, h.symbol, side, price, h_base.abs(), true));
            }
        }
        out
    }

    /// What a close is priced off: a fresh book that agrees with the mark, else the mark, else the
    /// position's entry price; capped around that reference.
    fn close_price(
        book: Option<&OrderBook>,
        age_ms: Option<u64>,
        side: Side,
        mark: Option<Decimal>,
        entry: Option<Decimal>,
        slip_bps: Decimal,
    ) -> Option<Decimal> {
        let (reference, base) = close_reference(book, age_ms, side, mark, STALE_MS, MAX_IOC_BPS)
            .or_else(|| entry.filter(|e| *e > Decimal::ZERO).map(|e| (e, e)))?;
        capped_ioc_price(side, base, reference, slip_bps, MAX_IOC_BPS)
    }

    fn place(
        &mut self,
        leg: &str,
        symbol: &str,
        side: Side,
        price: Decimal,
        qty: Decimal,
        reduce_only: bool,
    ) -> Intent {
        let id = self.id();
        let order = NewOrder {
            symbol: symbol.to_owned(),
            side,
            order_type: OrderType::Market,
            price,
            quantity: qty,
            client_id: Some(self.ids.issue()),
            reduce_only,
            protect: None,
        };
        Intent::Request { id, leg: leg.into(), call: VenueCall::Place(vec![order]) }
    }
}

impl StrategyAdapter for FundingArb {
    type Report = ArbReport;

    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        let Some(h) = hedge(view) else {
            return Err(StrategyError(
                "funding-arb: the hedge leg needs exactly one symbol (legs.hedge.symbols)".into(),
            ));
        };
        view.symbol.clone_into(&mut self.report.symbol);
        self.report.hedge_symbol = Some(h.symbol.to_owned());
        Ok(Vec::new())
    }

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent> {
        match event {
            Event::Book => self.book_ms = Some(view.now_ms),
            Event::Mark(m) if *m > Decimal::ZERO => self.mark = Some((*m, view.now_ms)),
            Event::Answer { id, reply } => return self.on_answer(*id, reply, view),
            Event::Tick => {
                let blocked = !matches!(self.flow, Flow::Idle) || view.now_ms < self.next_at_ms;
                if let (false, Some(h)) = (blocked, hedge(view)) {
                    return self.start_round(view, &h);
                }
            }
            _ => {}
        }
        Vec::new()
    }

    /// Flatten both legs through the bot's own flatten.
    fn stop(&mut self, view: &View<'_>) -> Vec<Intent> {
        let mut out = vec![Intent::Leg {
            leg: TRADE_LEG.into(),
            intent: Box::new(Intent::Flatten { symbols: vec![view.symbol.to_owned()] }),
        }];
        if let Some(h) = hedge(view) {
            out.push(Intent::Leg {
                leg: HEDGE_LEG.into(),
                intent: Box::new(Intent::Flatten { symbols: vec![h.symbol.to_owned()] }),
            });
        }
        out
    }

    fn report(&self) -> ArbReport {
        self.report.clone()
    }
}

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod tests;
