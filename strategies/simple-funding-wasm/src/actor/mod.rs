//! The strategy's own rules, ported from the built-in's execution actor onto the world's calls:
//! read the venue every tick (a failed read trades nothing), refresh the signals, fees and open
//! interest, [`decide`](crate::decide::decide), clamp again with [`plan`](crate::plan::plan), place
//! IOC orders on the `universe` leg as short/long pairs, unwind a one-sided pair, settle ambiguous
//! submits, watch the books' balance, stop on a daily loss or a high margin ratio.
//!
//! What it cannot be, on the `bb:strategy@0.5.0` world, and what it does instead:
//! - Its state is in memory. A restart forgets the day's loss baseline, a halt, the symbols it
//!   opened (`owned`) and the entry costs; the built-in saves them. After a restart over open
//!   positions it does not trade until `adopt_existing_positions` is set (they are not known to be
//!   its own), and the loss allowance starts again from the equity then.
//! - No alerts: the world has no notification intent. Unwinds, ambiguous orders and halts are in
//!   the report ([`crate::report::Status`]) and the bot's logs of the report.
//! - No owner "resume" of a halt: a halt lasts to the next UTC day (a restart lifts it too).
//! - Wind-down is one `Flatten` intent per held symbol, not the built-in's five retries with a
//!   widening cross: the bot's host carries out and retries the flatten.
//! - No paper mode, margin or fills of its own: whether it trades paper is the bot's.
//! - Time is the view's `now_ms`; the funding signal, open interest and fees are venue reads
//!   through requests, one at a time, so a tick takes several round trips.

mod data;
mod orders;
mod pair;
mod tick;
mod unwind;

use std::collections::{BTreeMap, BTreeSet};

use bb_strategy_sdk::{Decimal, Position, Side};
use bb_strategy_support::ClientIdIssuer;

use crate::config::SimpleFundingSettings;
use crate::decide::Params;
use crate::io::Io;
use crate::plan::Caps;

/// The leg the universe is on.
pub const UNIVERSE: &str = "universe";
/// The leg's venue.
pub const VENUE: &str = "bullet";

pub(crate) const DAY_MS: u64 = 86_400_000;

/// What a submit told us about an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sent {
    /// The venue accepted it (an IOC may still have filled none of it).
    Took,
    /// The venue refused it, or it never left the host.
    Rejected,
    /// A timeout, a transport error or no answer: it may have committed.
    Ambiguous,
}

#[derive(Debug, Default)]
pub(crate) struct SymState {
    pub signal: Option<Decimal>,
    pub signal_at_ms: Option<u64>,
    pub signal_next_fetch_ms: Option<u64>,
    pub fee_bps: Option<Decimal>,
    pub oi_base: Option<Decimal>,
    pub oi_at_ms: Option<u64>,
    pub step: Option<Decimal>,
    /// The round trip this symbol's open position was entered on (fraction of notional).
    pub entry_cost: Option<Decimal>,
}

/// A leg this strategy opened that must be closed again, until the venue shows it closed.
#[derive(Debug, Clone)]
pub(crate) struct Unwind {
    pub side: Side,
    pub qty: Decimal,
    pub attempts: u32,
    pub next_at_ms: u64,
}

/// An opening order whose commit is unknown.
#[derive(Debug, Clone)]
pub(crate) struct PendingLeg {
    pub symbol: String,
    pub side: Side,
    /// The signed position before the order.
    pub before: Decimal,
    pub price: Decimal,
}

/// Legs of one pair (or a lone order) whose submit was ambiguous, judged together once the venue
/// has had time to settle.
#[derive(Debug, Clone)]
pub(crate) struct Pending {
    pub legs: Vec<PendingLeg>,
    pub since_ms: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Day {
    pub index: u64,
    pub start_equity: Decimal,
}

/// The strategy's state between ticks. A flow owns it while it runs.
pub struct Core {
    pub(crate) config: SimpleFundingSettings,
    pub(crate) io: Io,
    pub(crate) syms: BTreeMap<String, SymState>,
    pub(crate) client_ids: ClientIdIssuer,
    pub(crate) halted: bool,
    pub(crate) day: Option<Day>,
    pub(crate) equity: Option<Decimal>,
    pub(crate) fee_refresh_last_ms: Option<u64>,
    /// The last tick's universe positions, signed base.
    pub(crate) held: BTreeMap<String, Decimal>,
    /// Symbols this strategy opened and has not seen flat since.
    pub(crate) owned: BTreeSet<String>,
    pub(crate) imbalance_ticks: u32,
    /// Why it does not trade (a start over positions it did not open).
    pub(crate) refused: Option<String>,
    /// The first tick has checked the account for positions it did not open.
    pub(crate) checked_start: bool,
    pub(crate) unwinds: BTreeMap<String, Unwind>,
    pub(crate) pending: Vec<Pending>,
    /// No entries until a tick reads the book balanced (a start over positions).
    pub(crate) recovering: bool,
    pub(crate) gross_usd: Decimal,
}

impl Core {
    /// A strategy over `symbols` (the universe, sized by `steps`), its ids seeded from `now_ms`.
    #[must_use]
    pub fn new(
        config: SimpleFundingSettings,
        io: Io,
        steps: &BTreeMap<String, Decimal>,
        now_ms: u64,
    ) -> Self {
        let syms = steps
            .iter()
            .map(|(symbol, step)| {
                (symbol.clone(), SymState { step: Some(*step), ..SymState::default() })
            })
            .collect();
        Self {
            config,
            io,
            syms,
            client_ids: ClientIdIssuer::own(now_ms),
            halted: false,
            day: None,
            equity: None,
            fee_refresh_last_ms: None,
            held: BTreeMap::new(),
            owned: BTreeSet::new(),
            imbalance_ticks: 0,
            refused: None,
            checked_start: false,
            unwinds: BTreeMap::new(),
            pending: Vec::new(),
            recovering: false,
            gross_usd: Decimal::ZERO,
        }
    }

    pub(crate) fn params(&self) -> Params {
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

    pub(crate) fn caps(&self) -> Caps {
        let c = &self.config;
        Caps {
            notional_per_symbol: c.notional_per_symbol,
            max_positions: c.max_positions,
            max_leverage: c.max_leverage,
            max_oi_pct: c.max_oi_pct,
        }
    }

    pub(crate) fn in_universe(&self, symbol: &str) -> bool {
        self.syms.contains_key(symbol)
    }

    pub(crate) fn now_ms(&self) -> u64 {
        self.io.now_ms()
    }

    /// Entries wait while a leg's commit is unsettled, an unwind is outstanding, or the start is
    /// still recovering a book it found lopsided.
    pub(crate) fn entries_blocked(&self) -> bool {
        !self.unwinds.is_empty() || !self.pending.is_empty() || self.recovering
    }

    /// Publish the state for the report.
    pub fn publish(&self) {
        let status = crate::report::Status {
            halted: self.halted,
            entries_blocked: self.entries_blocked(),
            refused: self.refused.clone(),
            unwinds: self
                .unwinds
                .iter()
                .map(|(k, u)| (k.clone(), format!("{} {} attempts {}", u.qty, k, u.attempts)))
                .collect(),
            pending_commits: self.pending.len(),
            markets: self.syms.len(),
            equity: self.equity,
            day_start_equity: self.day.map(|d| d.start_equity),
            gross_usd: self.gross_usd,
            positions: self.held.clone(),
            signals: self.syms.iter().filter_map(|(k, s)| Some((k.clone(), s.signal?))).collect(),
        };
        self.io.with(|s| s.status = status);
    }
}

/// A position's mark as the venue's unrealised `PnL` implies it (`entry` +/- `pnl / size`); the
/// entry price when the `PnL` is unusable.
pub(crate) fn position_mark(p: &Position) -> Decimal {
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

/// `qty x price`, saturating so absurd data reads as huge, not as a panic.
pub(crate) fn usd(qty: Decimal, price: Decimal) -> Decimal {
    qty.checked_mul(price).unwrap_or(if qty < Decimal::ZERO { Decimal::MIN } else { Decimal::MAX })
}

/// How far a position grew in `side`'s direction between two signed sizes.
pub(crate) fn grown(side: Side, before: Decimal, after: Decimal) -> Decimal {
    let delta = after.saturating_sub(before);
    match side {
        Side::Buy => delta,
        Side::Sell => Decimal::ZERO.saturating_sub(delta),
    }
}

impl Core {
    /// `start` read the venue's positions: whether the account holds universe positions, which
    /// were adopted, so the book must be read balanced before any entry. Without a read, the
    /// first tick checks.
    pub fn mark_start_checked(&mut self, over_positions: bool) {
        self.checked_start = true;
        self.recovering = over_positions;
    }
}
