//! The execution actor: reads the venue every tick, builds the [`MarketView`], runs
//! `decide`, clamps the result again ([`plan`]) and places IOC orders on Bullet.
//!
//! Safety rules, all in this module or [`plan`]:
//! - the venue's positions are re-read every tick and are the truth; a failed read trades nothing
//!   (it never reads as flat);
//! - it has no paper mode of its own: on paper the host gives it the shared paper broker;
//! - a disconnect requests shutdown, and `wind_down` always flattens everything it holds;
//! - a drop of equity past `daily_loss_usd` since the UTC day start flattens and halts until the
//!   next UTC day or an owner [`Controls::resume`]; the day's baseline and the halt survive a
//!   restart through the [`DayStateRepo`];
//! - a margin ratio at [`LIQ_RATIO_FLATTEN`] flattens and halts; an unknown ratio blocks entries;
//! - entries go in as long/short pairs, the first leg is unwound when the second fails, and a book
//!   left lopsided past `max_net_pct` for [`IMBALANCE_TICKS`] ticks sheds its weakest heavy leg;
//! - an entry whose book walk does not fit inside the IOC cap is not placed, and every IOC is
//!   priced within [`MAX_IOC_BPS_FROM_MID`] of a live book's mid (entries need that live book);
//! - an ambiguous submit (timeout, no answer) is read back from the venue and reconciled before any
//!   further entry; an unwind that fails stays outstanding, is retried with backoff and alerted,
//!   and blocks new pairs until the venue shows the leg closed;
//! - a live start refuses positions the bot did not open (`adopt_existing_positions`);
//! - no entries on stale data, unknown equity, margin, fee, open interest or signal.
//!
//! [`MarketView`]: crate::decide::MarketView
//! [`plan`]: crate::plan::plan
//! [`LIQ_RATIO_FLATTEN`]: crate::limits::LIQ_RATIO_FLATTEN
//! [`IMBALANCE_TICKS`]: crate::limits::IMBALANCE_TICKS
//! [`MAX_IOC_BPS_FROM_MID`]: crate::limits::MAX_IOC_BPS_FROM_MID

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal USD notionals, prices, sizes and bps sit far below 7.9e28; ms timestamps and the validated 1..=480 h window stay far inside i64"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bb_strategy_sdk::{OrderBook, Position, Side};
use bb_strategy_support::ClientIdIssuer;
use rust_decimal::Decimal;

use crate::actor::day::DayState;
use crate::adapter::io::{Mono, Shared};
use crate::config::SimpleFundingConfig;
use crate::decide::Params;
use crate::limits::FLATTEN_RETRY_WAIT;
use crate::plan::Caps;

/// The one venue.
pub const VENUE: &str = "bullet";

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: u64 = 86_400_000;

/// A position's mark as the venue's unrealised `PnL` implies it (`entry` +/- `pnl / size`); the
/// entry price when the `PnL` is unusable. Positions outside the universe get no live mark.
fn position_mark(p: &Position) -> Decimal {
    if p.size <= Decimal::ZERO {
        return p.entry_price;
    }
    let Some(per_unit) = p.unrealized_pnl.checked_div(p.size) else { return p.entry_price };
    let mark = match p.side {
        Some(Side::Buy) => p.entry_price.checked_add(per_unit),
        Some(Side::Sell) => p.entry_price.checked_sub(per_unit),
        None => None,
    };
    mark.filter(|m| *m > Decimal::ZERO).unwrap_or(p.entry_price)
}

/// `qty x price`, saturating at `Decimal::MAX` so absurd data reads as huge, not as a panic.
fn usd(qty: Decimal, price: Decimal) -> Decimal {
    qty.checked_mul(price).unwrap_or(if qty < Decimal::ZERO { Decimal::MIN } else { Decimal::MAX })
}

/// How far a position grew in `side`'s direction between two signed sizes.
fn grown(side: Side, before: Decimal, after: Decimal) -> Decimal {
    let delta = after - before;
    match side {
        Side::Buy => delta,
        Side::Sell => -delta,
    }
}

/// The owner's handle on a running actor: read the halt, lift it.
#[derive(Debug, Clone, Default)]
pub struct Controls {
    halted: Arc<AtomicBool>,
    resume: Arc<AtomicBool>,
}

impl Controls {
    /// The daily-loss stop is holding the bot flat.
    #[must_use]
    pub fn is_halted(&self) -> bool {
        self.halted.load(Ordering::SeqCst)
    }

    /// Lift the daily-loss halt at the next tick. The loss allowance restarts from the
    /// equity at that moment.
    pub fn resume(&self) {
        self.resume.store(true, Ordering::SeqCst);
    }
}

#[derive(Debug, Default)]
struct SymState {
    mark: Option<Decimal>,
    mark_at: Option<Mono>,
    /// The live funding rate per 8h, for display only: the signal is settled funding.
    live_funding: Option<Decimal>,
    book: Option<Arc<OrderBook>>,
    book_at: Option<Mono>,
    signal: Option<Decimal>,
    signal_at: Option<Mono>,
    signal_next_fetch: Option<Mono>,
    fee_bps: Option<Decimal>,
    oi_base: Option<Decimal>,
    oi_at: Option<Mono>,
    step: Option<Decimal>,
    /// The round trip this symbol's open position was entered on (fraction of notional): the
    /// exit hurdle hangs off it so the book widening later does not flap the pair. Lost on a
    /// restart (the hurdle then reads today's cost).
    entry_cost: Option<Decimal>,
}

/// What a submit told us about an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sent {
    /// The venue accepted it (an IOC may still have filled none of it).
    Took,
    /// The venue refused it, or it never left this process.
    Rejected,
    /// A timeout, a transport error or no answer: it may have committed. Treated as possibly
    /// filled until the venue's position says otherwise.
    Ambiguous,
}

/// A leg this bot opened that must be closed again, until the venue shows it closed.
#[derive(Debug, Clone)]
struct Unwind {
    /// The side of the leg being closed (the close is the opposite side).
    side: Side,
    /// Up to this much of the leg is closed.
    qty: Decimal,
    attempts: u32,
    next_at: Mono,
}

/// An opening order whose commit is unknown.
#[derive(Debug, Clone)]
struct PendingLeg {
    symbol: String,
    side: Side,
    /// The signed position before the order.
    before: Decimal,
    price: Decimal,
}

/// Legs of one pair (or a lone order) whose submit was ambiguous, judged together once the
/// venue has had time to settle.
#[derive(Debug, Clone)]
struct Pending {
    legs: Vec<PendingLeg>,
    since: Mono,
}

#[derive(Debug, Clone, Copy)]
struct Day {
    index: u64,
    start_equity: Decimal,
}

/// Whose daily loss stop runs: the actor's own, or (behind the host) the host's loss watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LossStop {
    Own,
    Host,
}

#[expect(clippy::struct_excessive_bools, reason = "independent facts of one run, not a state")]
pub struct SimpleFundingActor {
    config: SimpleFundingConfig,
    symbols: Vec<String>,
    syms: BTreeMap<String, SymState>,
    client_ids: ClientIdIssuer,
    controls: Controls,
    day: Option<Day>,
    equity: Option<Decimal>,
    fee_refresh_last: Option<Mono>,
    /// The last tick's venue positions in the universe, signed base, for `status`.
    held: BTreeMap<String, Decimal>,
    retry_wait: Duration,
    /// The day's baseline, the halt and `owned` are saved (the host's store keeps them across a
    /// restart), and what the last run saved (or why it cannot be read), taken at the start.
    persists: bool,
    saved_day: Option<Result<Option<DayState>, String>>,
    /// Its own daily loss stop runs; off behind the host, whose loss watch is the one stop.
    loss_stop: LossStop,
    /// Symbols this bot opened and has not seen flat since.
    owned: BTreeSet<String>,
    /// Ticks in a row the books sat past `max_net_pct`.
    imbalance_ticks: u32,
    /// `init` refused the start: `wind_down` must not touch positions that are not the bot's.
    refused: bool,
    /// The actor clock's `now` as of the tick (or flatten) in progress.
    now: Option<Mono>,
    /// What the flows ask of the host besides venue calls: alerts and saved state.
    io: Shared,
    /// Legs being closed again, by symbol. In memory: the venue's positions are the truth, and
    /// a restart finds the same legs through `recovering`.
    unwinds: BTreeMap<String, Unwind>,
    /// Ambiguous commits not yet reconciled.
    pending: Vec<Pending>,
    /// A live start does not trust a book it did not see being built: no entries until a tick
    /// reads it balanced, and a lopsided one is shed at once.
    recovering: bool,
    /// Where each tick's [`report::SimpleFundingReport`] goes (the instance's snapshot).
    report_sink: Option<Arc<dyn report::ReportSink>>,
    /// The universe's gross exposure as the last tick read it (USD).
    gross_usd: Decimal,
    /// A tick of this run read both the positions and the equity: until one has, nothing is
    /// reported (a rebuilt instance's carried account numbers are not overwritten by unread ones).
    read_once: bool,
}

impl SimpleFundingActor {
    /// An actor over `symbols`; `init` keeps those Bullet can size.
    #[must_use]
    pub fn new(config: SimpleFundingConfig, symbols: Vec<String>) -> Self {
        Self {
            config,
            symbols,
            syms: BTreeMap::new(),
            client_ids: ClientIdIssuer::session_seeded(),
            controls: Controls::default(),
            day: None,
            equity: None,
            fee_refresh_last: None,
            held: BTreeMap::new(),
            retry_wait: FLATTEN_RETRY_WAIT,
            persists: false,
            saved_day: None,
            loss_stop: LossStop::Own,
            owned: BTreeSet::new(),
            imbalance_ticks: 0,
            refused: false,
            now: None,
            io: Shared::default(),
            unwinds: BTreeMap::new(),
            pending: Vec::new(),
            recovering: false,
            report_sink: None,
            gross_usd: Decimal::ZERO,
            read_once: false,
        }
    }

    /// Keep the day's baseline, the halt and the owned symbols (saved as [`KEY_DAY_STATE`] writes),
    /// so a restart or rebuild resumes them instead of re-baselining on the post-loss equity.
    /// `saved` is what the store holds now, read by the caller: `Err` (an unreadable record)
    /// refuses the start, `Ok(None)` is a first run.
    ///
    /// [`KEY_DAY_STATE`]: day::KEY_DAY_STATE
    #[must_use]
    pub fn with_saved_day(mut self, saved: Result<Option<DayState>, String>) -> Self {
        self.persists = true;
        self.saved_day = Some(saved);
        self
    }

    /// The channel the flows' alerts and writes leave on (the host's poller drains it).
    pub(crate) fn shared(&self) -> Shared {
        self.io.clone()
    }

    /// Behind the strategy host: the host's loss watch is the one daily loss stop (its limit
    /// already takes this strategy's `daily_loss_usd`), so this one's is off. The margin halt
    /// stays.
    #[must_use]
    pub fn without_own_loss_stop(mut self) -> Self {
        self.loss_stop = LossStop::Host;
        self
    }

    /// The halt flag and the resume switch.
    #[must_use]
    pub fn controls(&self) -> Controls {
        self.controls.clone()
    }

    /// Wait this long between flatten attempts (tests shorten it).
    #[must_use]
    pub fn with_retry_wait(mut self, wait: Duration) -> Self {
        self.retry_wait = wait;
        self
    }

    fn halted(&self) -> bool {
        self.controls.is_halted()
    }

    fn params(&self) -> Params {
        let c = &self.config;
        Params {
            notional_per_symbol: c.notional_per_symbol,
            top_k: c.top_k,
            max_positions: c.max_positions,
            max_net_pct: c.max_net_pct,
            max_oi_pct: c.max_oi_pct,
            max_leverage: c.max_leverage,
            expected_hold_hours: c.expected_hold_hours,
            min_depth_usd: c.min_depth_usd,
        }
    }

    fn caps(&self) -> Caps {
        let c = &self.config;
        Caps {
            notional_per_symbol: c.notional_per_symbol,
            max_positions: c.max_positions,
            max_leverage: c.max_leverage,
            max_oi_pct: c.max_oi_pct,
        }
    }

    fn in_universe(&self, symbol: &str) -> bool {
        self.syms.contains_key(symbol)
    }

    /// The price a position is valued at: the live mark, else the mark the venue's own
    /// unrealised `PnL` implies, else its entry price.
    fn valuation(&self, p: &Position) -> Decimal {
        self.syms
            .get(&p.symbol)
            .and_then(|s| s.mark)
            .filter(|m| *m > Decimal::ZERO)
            .unwrap_or_else(|| position_mark(p))
    }

    fn fresh(&self, s: &SymState, now: Mono) -> bool {
        let limit = Duration::from_secs(self.config.stale_secs);
        let recent = |at: Option<Mono>| {
            at.is_some_and(|t| now.checked_duration_since(t).is_some_and(|age| age <= limit))
        };
        recent(s.mark_at) && recent(s.book_at)
    }
}

pub mod day;
mod flows;
mod loss_stop;
mod market_data;
mod orders;
pub mod report;
