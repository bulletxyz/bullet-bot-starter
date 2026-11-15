//! The [`Intent::Targets`](crate::Intent::Targets) intent: what a [`TargetStrategy`] sees (a
//! [`MarketView`], built by the bot's executor each decision tick) and what it returns (one
//! [`Target`] per symbol). The bot's executor clamps every target to its caps and turns it into
//! hedged orders; a strategy gets no broker, and that it places no orders of its own is a
//! convention the type does not enforce.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// A funding strategy: hedged target positions per symbol across Bullet and Hyperliquid.
///
/// `decide` gets the current [`MarketView`] and returns one [`Target`] per symbol in the
/// view, in view order (a symbol left out is read as target 0). By convention it does no
/// I/O: nothing stops an implementation (a closure included) from capturing a client and
/// trading directly, and those orders bypass every cap. The targets it returns go through
/// the executor's clamps (caps, leverage, open interest, one grow in flight), so a bug in
/// them can size a position wrongly but cannot exceed a limit.
pub trait TargetStrategy: Send + Sync + 'static {
    fn decide(&mut self, view: &MarketView) -> Vec<Target>;
}

impl<F> TargetStrategy for F
where
    F: FnMut(&MarketView) -> Vec<Target> + Send + Sync + 'static,
{
    fn decide(&mut self, view: &MarketView) -> Vec<Target> {
        self(view)
    }
}

/// Which venue is short. `ShortBullet` = short Bullet / long Hyperliquid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    ShortBullet,
    LongBullet,
}

/// One venue's current position in a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Leg {
    /// Signed base size: negative is short.
    pub base: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolView {
    /// Bullet symbol, e.g. `"BTC-USD"`.
    pub symbol: String,
    /// Trailing mean funding spread (Bullet − HL), per 8h. `None` = not enough history.
    pub signal_8h: Option<Decimal>,
    /// Round trip as a fraction of notional. `None` = unknown fee or uncostable book.
    pub round_trip_cost: Option<Decimal>,
    /// Bullet mark price; 0 = unknown.
    pub mark: Decimal,
    /// Bullet open interest in USD. `None` = unknown.
    pub bullet_oi_usd: Option<Decimal>,
    pub leg_bullet: Leg,
    pub leg_hl: Leg,
}

/// The config knobs `decide` uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Params {
    pub notional_per_symbol: Decimal,
    pub max_positions: usize,
    pub max_oi_pct: Decimal,
    pub max_leverage: Decimal,
    pub expected_hold_hours: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketView {
    pub symbols: Vec<SymbolView>,
    /// Account equity per venue, USD. `None` = unknown.
    pub equity_bullet: Option<Decimal>,
    pub equity_hl: Option<Decimal>,
    /// Notional of positions the bot doesn't own (unpaired or other symbols), USD.
    pub foreign_notional_bullet: Decimal,
    pub foreign_notional_hl: Decimal,
    pub params: Params,
}

/// Desired notional per leg for one symbol. 0 = flat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub symbol: String,
    pub direction: Direction,
    pub notional_usd: Decimal,
}
