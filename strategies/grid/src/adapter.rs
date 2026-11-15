//! The grid as a [`StrategyAdapter`](bb_strategy_sdk::StrategyAdapter): data in, intents out.
//!
//! Start: resolve the [`Plan`] (derived from the price history the build read, or the config's
//! overrides), refuse a worst case past the dollars or the hard limits, lay out the levels (the
//! one nearest the mid empty), open the starting position with one IOC (long, short) and place
//! every other level resting (post-only; a level that would cross goes at the touch). A long or
//! short grid holds only its own side: its closing levels go out as far as the position it holds
//! (none while flat at start), since no hard stop guards the other side.
//!
//! Events:
//!   - `Fill` — the host has already recorded it in the inventory. A completed level goes dormant
//!     and re-arms its neighbour with the opposite side (a buy fills: a sell one level up).
//!   - `Order` — learn exchange `order_ids`; mark cancel/reject levels Pending so the tick
//!     re-places them.
//!   - `Outcome` — the answer to a `Place`: the level goes Active, or stays Pending for the next
//!     pass; until it comes the level is in flight and not placed again. An order answered after its
//!     level let go is cancelled. A refused starting IOC stops the bot. A cancel-all that failed is
//!     asked again on every tick until one lands ([`Pull`]). An acked level is freed only as an
//!     unknown one is.
//!   - `Failed` — a `Place` the host never sent: taken as refused (a starting IOC stops the bot).
//!   - `Unknown` — a `Place` the venue left unanswered holds its level as placed; a read at least
//!     the settle margin later that does not list it frees the level.
//!   - `Book` — the hard stop and the daily loss stop are checked on every book.
//!   - `Tick` — the stops, the out-of-range pause, the trend filter, the missing-order reconcile,
//!     the cancel of an order let go of that still rests (`adapter/orders/stray.rs`), the
//!     pending placements; once stopped, the close is retried until flat.
//!
//! Safety (money-safety code: a failing test first):
//!   1. The worst case (every risk-side level filled) must fit the dollars and the hard limits
//!      ([`crate::plan`]).
//!   2. Past the hard stop (`stop_beyond` range widths past the losing edge): cancel all, close
//!      with reduce-only IOCs (the order path sizes them to the venue position) and stop.
//!   3. Out of range on the profitable side (long above the range, short below it): the opening
//!      orders are cancelled until the price is back; on the losing side the grid holds.
//!   4. The trend filter cancels the opening orders only; the daily loss stop flattens and stops
//!      like the hard stop.
//!   5. The report shows the host's realized and unrealized P&L of the whole position.
//!
//! Unlike the recorded built-in (its parity fixtures edited, not re-recorded): an in-flight level is not placed
//! again, a directional start has no closing level, and a read within the settle margin of an ack frees nothing.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal USD notionals, prices, sizes and bps sit far below 7.9e28; skip/placement \
              counters and close attempts are tiny"
)]

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bb_strategy_sdk::{CancelOrder, Intent, StrategyError};
use bb_strategy_support::ClientIdIssuer;
use rust_decimal::Decimal;

use crate::adapter::stops::pull::Pull;
use crate::config::GridConfig;
use crate::levels::{GridState, LevelState};
use crate::plan::{HardLimits, History, Plan};

mod events;
mod orders;
mod pauses;
mod port;
mod reclaim;
pub mod stops;
mod warm;

/// How long after a place's answer, or a cancel the grid sent, a read must start before what it omits or lists
/// is believed: the venue's REST can lag.
const SETTLE_MS: u64 = 2_000;

/// Whether the build's daily loss stop has tripped (the order path's stop, read by the build).
pub type DayStop = Arc<dyn Fn() -> bool + Send + Sync>;

/// Holds the instance for safety on the hard stop, through a restart until the owner resumes: never
/// a new grid in the market it just stopped out of.
pub type HardStopHold = Arc<dyn Fn() + Send + Sync>;

/// The alert that says the hard stop holds the grid.
pub const KEY_HARD_STOP: &str = "grid_hard_stop";

/// How far past the touch the starting IOC may fill (bps).
pub const ENTRY_SLIP_BPS: Decimal = Decimal::from_parts(20, 0, 0, false, 0);
/// How much further past the touch each close attempt may fill (bps).
pub const CLOSE_SLIP_BPS: Decimal = Decimal::from_parts(20, 0, 0, false, 0);

/// Why the grid stopped trading, and how many closes it sent since.
#[derive(Debug, Clone)]
struct Halt {
    reason: String,
    attempts: u32,
    /// The cancel the stop asks, and asks again while it has not landed: the hard stop's takes
    /// only the bot's own orders, the daily loss stop's every order on the market.
    pull: Intent,
}

/// What the last view showed: the report's P&L.
#[derive(Debug, Clone, Default)]
struct Seen {
    mid: Option<Decimal>,
    position: Decimal,
    avg_entry: Option<Decimal>,
    realized: Decimal,
}

pub struct GridStrategy {
    config: GridConfig,
    history: Option<History>,
    limits: HardLimits,
    day_stop: Option<DayStop>,
    hard_stop_hold: Option<HardStopHold>,
    plan: Option<Plan>,
    tick: Option<Decimal>,
    state: GridState,
    client_ids: ClientIdIssuer,
    /// The starting position's IOC: its client id until the venue answers it.
    entry: Option<String>,
    range_paused: bool,
    halt: Option<Halt>,
    seen: Seen,
    /// The last cancel-all, and whether it failed (see [`Pull`]).
    pull: Pull,
    /// Levels held for a place the venue acked or left unknown: the client id and the bot clock
    /// (ms) of the answer. A read frees one only if it is at least [`SETTLE_MS`] later.
    placed_at: HashMap<String, u64>,
    /// Orders let go of before the grid saw them gone, with when (`adapter/orders/stray.rs`); sorted so their
    /// cancels go out in client id order.
    let_go: BTreeMap<String, u64>,
    /// How the trend filter's warm-up went, for the briefing (`adapter/warm.rs`).
    warmup_note: Option<String>,
    /// When the grid first saw the market with that note up (`adapter/warm.rs`).
    note_since: Option<u64>,
    /// This start takes over the position a kept shutdown left (`instance.on_shutdown = keep`,
    /// `adapter/reclaim.rs`).
    reclaiming: bool,
    /// A directional or reclaimed grid's caps, long and short (coins): its plan's worst case on each side, zero
    /// on the side a long or short grid's stop leaves unguarded. `None`: the grid's own cap on both sides.
    side_caps: Option<(Decimal, Decimal)>,
}

impl GridStrategy {
    pub fn new(config: GridConfig) -> Self {
        let state = match &config.trend_filter {
            Some(cfg) => GridState::new().with_trend_filter(cfg),
            None => GridState::new(),
        };
        Self {
            config,
            history: None,
            limits: HardLimits::default(),
            day_stop: None,
            hard_stop_hold: None,
            plan: None,
            tick: None,
            state,
            client_ids: ClientIdIssuer::session_seeded(),
            entry: None,
            range_paused: false,
            halt: None,
            seen: Seen::default(),
            pull: Pull::default(),
            placed_at: HashMap::new(),
            let_go: BTreeMap::new(),
            warmup_note: None,
            note_since: None,
            reclaiming: false,
            side_caps: None,
        }
    }

    /// The price history the range and the step are derived from (the build reads it).
    #[must_use]
    pub fn with_history(mut self, history: History) -> Self {
        self.history = Some(history);
        self
    }

    /// The config's hard limits: the worst case is refused past them.
    #[must_use]
    pub fn with_limits(mut self, limits: HardLimits) -> Self {
        self.limits = limits;
        self
    }

    /// The build's daily loss stop: when it trips the grid flattens and stops.
    #[must_use]
    pub fn with_day_stop(mut self, day_stop: DayStop) -> Self {
        self.day_stop = Some(day_stop);
        self
    }

    /// The hard stop holds the instance through `hold` ([`HardStopHold`]).
    #[must_use]
    pub fn with_hard_stop_hold(mut self, hold: HardStopHold) -> Self {
        self.hard_stop_hold = Some(hold);
        self
    }

    fn symbol(&self) -> &str {
        &self.config.symbol
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
        // A product past `Decimal::MAX` is a floor no spacing meets: refused, never a panic.
        let required_bps = (Decimal::from(2) * fees.maker_bps)
            .checked_mul(fees.min_spacing_fee_multiplier)
            .ok_or_else(|| StrategyError("The grid's fee floor is past any spacing".into()))?;
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

    fn resume_if_clear(&mut self) {
        if !self.paused() {
            self.state.unpause_all();
        }
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
}

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod tests;
