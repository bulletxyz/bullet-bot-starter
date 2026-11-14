//! An example strategy written only against `bb-strategy-sdk`: a ladder of `levels` buys and sells
//! around the mid, `spacing_bps` apart, placed at start (or on the first book). A filled level is
//! placed again at its price. A level the venue or the host did not keep is placed again only
//! where that is right (`retry.rs`): a post-only cross, a stop that lifts by itself and a cancel by
//! someone else come back on a later book update that no longer crosses them; a limit is final. A
//! placement with no answer, or an unknown one, is held until a fresh venue read that began
//! [`UNLISTED_AFTER_MS`] after it lists it (it rests) or omits it (placed again on a later book).
//! At stop it cancels everything. Each level is a ledger of its orders with one transition function
//! (`level.rs`). Built for `wasm32-wasip2` it is a `bb:strategy` component (`just wasm`).

use bb_strategy_sdk::{
    Decimal, Describe, Descriptor, Event, FromSettings, Intent, Loosening, NewOrder, OrderStatus,
    OrderType, PaperInputs, Range, ReportField, SettingKind, SettingSpec, Side, StrategyAdapter,
    StrategyError, StrategyReport, Tier, View,
};
use serde::{Deserialize, Serialize};

mod ladder;
mod level;
mod retry;

use level::{Action, Cx, Ended, Ev, Level};

/// What the host's refusal says first when orders are stopped (`bb_strategy::STOPPED_REFUSAL`).
const STOPPED_PHRASE: &str = "new orders are stopped";
/// What the host's refusal for the order rate says (`bb_strategy::RATE_REFUSAL`).
const RATE_PHRASE: &str = "the order rate limit is reached";

pub use ladder::{LevelState, Rung};

/// The ladder's settings, as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LadderSettings {
    /// Levels on each side.
    pub levels: u32,
    /// Distance between levels (and from the mid to the first), in basis points of the mid.
    pub spacing_bps: Decimal,
    pub order_size: Decimal,
    #[serde(default = "post_only")]
    pub order_type: OrderType,
}

fn post_only() -> OrderType {
    OrderType::PostOnly
}

/// How long after its send a placement with no answer (or an unknown one) is held before a venue
/// read that began then and omits it frees the level: the host counts an order it may have sent
/// for 30 s, and a venue lists one long before.
pub const UNLISTED_AFTER_MS: u64 = 30_000;

/// Most levels a side may have: 64 orders, the most a host takes from one call.
pub const MAX_LEVELS: u32 = 32;

impl LadderSettings {
    fn check(&self) -> Result<(), StrategyError> {
        let bad = |why: &str| Err(StrategyError(format!("ladder: {why}")));
        if self.levels == 0 || self.levels > MAX_LEVELS {
            return bad(&format!("levels must be 1 to {MAX_LEVELS}, got {}", self.levels));
        }
        if self.spacing_bps <= Decimal::ZERO || self.spacing_bps >= Decimal::from(10_000) {
            return bad("spacing_bps must be above 0 and below 10000");
        }
        if self.order_size <= Decimal::ZERO {
            return bad("order_size must be above 0");
        }
        // The farthest buy is `levels` spacings below the mid: it must stay above zero.
        if Decimal::from(self.levels).saturating_mul(self.spacing_bps) >= Decimal::from(10_000) {
            return bad(
                "levels times spacing_bps must be below 10000: the farthest buy would be at or below zero",
            );
        }
        if self.order_type == OrderType::Market {
            return bad("order_type must rest (limit or post_only), not market");
        }
        Ok(())
    }
}

/// The ladder strategy.
#[derive(Debug, Clone)]
pub struct Ladder {
    settings: LadderSettings,
    levels: Vec<Level>,
    /// The mid the ladder was laid around.
    mid: Option<Decimal>,
    /// Completions counted (a level's fill reaching its size, once per cycle).
    fills: u64,
    /// Milliseconds with a level resting, and the last time the host called (the bot's clock).
    quoting_ms: u64,
    /// Whether a level rested at the last call.
    was_resting: bool,
    last_ms: u64,
    /// Cancels by someone else not yet counted: (when, level, on a usable book). Judged on the
    /// next book update, when it shows whether they were one level (the venue) or several (a
    /// sweep).
    pending: Vec<(u64, usize, bool)>,
}

impl Ladder {
    /// # Errors
    /// Settings it cannot run with.
    pub fn new(settings: LadderSettings) -> Result<Self, StrategyError> {
        settings.check()?;
        Ok(Self {
            settings,
            levels: Vec::new(),
            mid: None,
            fills: 0,
            quoting_ms: 0,
            was_resting: false,
            last_ms: 0,
            pending: Vec::new(),
        })
    }

    /// Feed `ev` to level `at` and turn what it decides into intents.
    fn run(&mut self, at: usize, ev: Ev<'_>, view: &View<'_>) -> Vec<Intent> {
        let usable = retry::usable(view.book);
        let Some(level) = self.levels.get_mut(at) else { return Vec::new() };
        let cx = Cx { now_ms: view.now_ms, usable, crosses: level.crosses(view.book) };
        let out = level.apply(ev, cx);
        self.fills = self.fills.saturating_add(u64::from(out.completions));
        if out.venue_cancel {
            self.pending.push((view.now_ms, at, usable));
        }
        let (side, price) = (level.side, level.price);
        out.actions
            .into_iter()
            .map(|action| match action {
                Action::Place { client_id, qty } => Intent::Place(NewOrder {
                    protect: None,
                    symbol: view.symbol.to_owned(),
                    side,
                    order_type: self.settings.order_type,
                    price,
                    quantity: qty,
                    client_id: Some(client_id),
                    reduce_only: false,
                }),
                Action::Cancel { client_id } => Intent::Cancel(bb_strategy_sdk::CancelOrder {
                    symbol: view.symbol.to_owned(),
                    order_id: String::new(),
                    client_id: Some(client_id),
                }),
            })
            .collect()
    }

    /// The level that has the order `client_id`.
    fn level_of(&self, client_id: Option<&str>) -> Option<usize> {
        let id = client_id?;
        self.levels.iter().position(|l| l.owns(id))
    }

    /// Lay the ladder around the view's usable book, once.
    fn lay(&mut self, view: &View<'_>) -> Vec<Intent> {
        if !self.levels.is_empty() || !retry::usable(view.book) {
            return Vec::new();
        }
        let Some(mid) = view.book.and_then(bb_strategy_sdk::OrderBook::midpoint) else {
            return Vec::new();
        };
        let mid = mid.normalize();
        let Some(levels) = ladder::levels(mid, &self.settings, view.market.tick_size) else {
            return Vec::new();
        };
        self.levels = levels;
        self.mid = Some(mid);
        (0..self.levels.len()).flat_map(|at| self.run(at, Ev::Lay, view)).collect()
    }

    /// Cancels by someone else on a usable book count toward a level's cap, unless several levels
    /// were cancelled together (a sweep).
    fn settle_cancels(&mut self, view: &View<'_>) {
        let pending = std::mem::take(&mut self.pending);
        for (t, at, usable) in &pending {
            let sweep =
                pending.iter().any(|(t2, a2, _)| a2 != at && t.abs_diff(*t2) <= retry::SWEEP_MS);
            if *usable && !sweep {
                self.run(*at, Ev::CountCancel, view);
            }
        }
    }

    fn note_time(&mut self, now_ms: u64) {
        if self.was_resting {
            self.quoting_ms = self.quoting_ms.saturating_add(now_ms.saturating_sub(self.last_ms));
        }
        self.last_ms = now_ms;
        self.was_resting = self.levels.iter().any(|l| l.rung().state == LevelState::Resting);
    }
}

impl Describe for Ladder {
    const NAME: &'static str = "ladder";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn describe() -> Descriptor {
        let d = |n: i64| Decimal::from(n);
        let order_type = SettingSpec {
            kind: SettingKind::Choice(vec!["limit".into(), "post_only".into()]),
            ..SettingSpec::int("order_type", 1, Range::new(d(0), d(1)), "How each level rests.")
        };
        Descriptor {
            summary:
                "Buys and sells spacing_bps apart around the mid; a filled level is placed again."
                    .into(),
            settings: vec![
                SettingSpec::int(
                    "levels",
                    3,
                    Range::new(d(1), d(i64::from(MAX_LEVELS))),
                    "Levels on each side.",
                )
                .required(),
                SettingSpec::decimal(
                    "spacing_bps",
                    d(10),
                    Range::new(Decimal::new(1, 2), d(9_999)),
                    "Distance between levels, in basis points of the mid.",
                )
                .required(),
                SettingSpec::decimal(
                    "order_size",
                    Decimal::new(1, 2),
                    Range::default(),
                    "Size of one order.",
                )
                .tier(Tier::Limit)
                .loosening(Loosening::Up)
                .required(),
                order_type,
            ],
            settings_schema: schemars::schema_for!(LadderSettings),
            legs: Vec::new(),
        }
    }
}

impl FromSettings for Ladder {
    fn from_settings(settings: &str) -> Result<Self, StrategyError> {
        let settings: LadderSettings = serde_json::from_str(settings)
            .map_err(|e| StrategyError(format!("ladder: settings: {e}")))?;
        Self::new(settings)
    }
}

impl StrategyAdapter for Ladder {
    type Report = LadderReport;

    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        self.last_ms = view.now_ms;
        Ok(self.lay(view))
    }

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent> {
        let intents = match event {
            Event::Book => {
                let mut intents = self.lay(view);
                if intents.is_empty() {
                    if retry::usable(view.book) {
                        self.settle_cancels(view);
                    }
                    intents = (0..self.levels.len())
                        .flat_map(|at| self.run(at, Ev::Book, view))
                        .collect();
                }
                intents
            }
            // A tick lays the ladder and, with a fresh venue read and its start, settles the levels
            // whose placement has no answer: nothing is placed again or asked again on it.
            Event::Tick => {
                let intents = self.lay(view);
                if let (Some(open), Some(started_ms)) = (view.open_orders, view.read_started_ms) {
                    for at in 0..self.levels.len() {
                        self.run(at, Ev::Read { open, started_ms }, view);
                    }
                }
                intents
            }
            Event::Fill(trade) => match self.level_of(trade.client_id.as_deref()) {
                Some(at) => {
                    let id = trade.client_id.as_deref().unwrap_or_default();
                    self.run(at, Ev::Trade { id, qty: trade.quantity }, view)
                }
                None => Vec::new(),
            },
            Event::Order(order) => {
                let by = match order.status {
                    OrderStatus::Cancelled => Some(Ended::Cancelled),
                    OrderStatus::Rejected => Some(Ended::Rejected),
                    OrderStatus::Filled => Some(Ended::Filled),
                    _ => None,
                };
                match (by, self.level_of(order.client_id.as_deref())) {
                    (Some(by), Some(at)) => {
                        let id = order.client_id.as_deref().unwrap_or_default();
                        self.run(at, Ev::Ended { id, by, filled: order.filled_quantity }, view)
                    }
                    _ => Vec::new(),
                }
            }
            Event::Outcome { intent: Intent::Place(order), result } => {
                let id = order.client_id.as_deref().unwrap_or_default();
                match self.level_of(Some(id)) {
                    Some(at) if result.success => self.run(at, Ev::PlaceOk { id }, view),
                    Some(at) => {
                        let crosses = self.levels.get(at).is_some_and(|l| l.crosses(view.book));
                        let why = retry::refusal_of(result.error.as_deref(), crosses);
                        self.run(at, Ev::PlaceRefused { id, why }, view)
                    }
                    None => Vec::new(),
                }
            }
            Event::Outcome { intent: Intent::Cancel(cancel), result } if !result.success => {
                let id = cancel.client_id.as_deref().unwrap_or_default();
                match self.level_of(Some(id)) {
                    Some(at) => self.run(at, Ev::CancelRefused { id }, view),
                    None => Vec::new(),
                }
            }
            // The order may rest or have filled: the level holds it (pending) until its events come
            // or a fresh read settles it (`Level::on_read`).
            Event::Unknown { .. }
            | Event::Outcome { .. }
            | Event::Mark(_)
            | Event::Leg { .. }
            | Event::Answer { .. } => Vec::new(),
        };
        self.note_time(view.now_ms);
        intents
    }

    fn stop(&mut self, view: &View<'_>) -> Vec<Intent> {
        self.last_ms = view.now_ms;
        vec![Intent::CancelAll]
    }

    fn report(&self) -> LadderReport {
        let quoting_secs = self.quoting_ms.checked_div(1_000).unwrap_or(0);
        LadderReport {
            mid: self.mid,
            levels: self.levels.iter().map(Level::rung).collect(),
            fills: self.fills,
            order_size: self.settings.order_size,
            quoting_secs,
        }
    }
}

/// What the ladder shows of itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct LadderReport {
    /// The mid the ladder was laid around; none before the first book.
    pub mid: Option<Decimal>,
    pub levels: Vec<Rung>,
    /// Levels filled (and placed again) so far.
    pub fills: u64,
    pub order_size: Decimal,
    /// Seconds with a level resting (for the paper report).
    pub quoting_secs: u64,
}

impl StrategyReport for LadderReport {
    fn headline(&self) -> String {
        let count = |state| self.levels.iter().filter(|r| r.state == state).count();
        match self.mid {
            None => "ladder: waiting for a book".into(),
            Some(mid) => {
                let held = count(LevelState::Held);
                let held = if held > 0 { format!(", {held} held") } else { String::new() };
                format!(
                    "ladder around {mid}: {} resting, {} pending, {} refused, {} fills{held}",
                    count(LevelState::Resting),
                    count(LevelState::Pending),
                    count(LevelState::Refused),
                    self.fills
                )
            }
        }
    }

    fn fields(&self) -> Vec<ReportField> {
        let mut fields: Vec<ReportField> =
            self.mid.map(|m| ReportField::number("mid", m)).into_iter().collect();
        fields.push(ReportField::number("fills", Decimal::from(self.fills)));
        fields.push(ReportField::number("order_size", self.order_size));
        fields
            .push(ReportField::number("quoting_secs", Decimal::from(self.quoting_secs)).unit("s"));
        fields.extend(self.levels.iter().map(Rung::field));
        fields
    }

    fn paper_inputs(&self) -> Option<PaperInputs> {
        Some(PaperInputs {
            order_size: self.order_size,
            quoting_secs: self.quoting_secs,
            maker_fee_bps: Decimal::ZERO,
        })
    }

    fn briefing_lines(&self) -> Vec<String> {
        let resting = |side| {
            self.levels.iter().filter(|r| r.side == side && r.state == LevelState::Resting).count()
        };
        vec![format!(
            "The ladder has {} resting buys and {} resting sells around the mid. ",
            resting(Side::Buy),
            resting(Side::Sell)
        )]
    }

    fn reference_price(&self) -> Option<Decimal> {
        self.mid
    }
}

bb_strategy_sdk::export_strategy!(Ladder);

#[cfg(test)]
mod prop_model;
#[cfg(test)]
mod prop_tests;
#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
