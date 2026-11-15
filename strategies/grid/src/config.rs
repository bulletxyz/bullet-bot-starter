use bb_strategy_sdk::OrderType;
use bb_strategy_sdk::Setting;
use bb_strategy_sdk::ValidateConfig;
use bb_strategy_sdk::param_spec::{Loosening, ParamSpec, Range, Tier, whole};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::derive::{ATR_PERIOD, Direction, RangePreset};

/// Configuration for the grid.
///
/// The owner gives three things: the dollars (`amount_usd`), the `direction` and the `range`
/// preset. The range, the step and the level count come from the market's price history at start
/// (see [`crate::derive`]); `lower_price`, `upper_price`, `num_levels` and `order_size` override
/// them for an advanced owner, and with all four given no history is read.
///
/// When a buy at level `N` fills the grid places a sell at `N+1`; when a sell at `M` fills it
/// places a buy at `M-1`. Profit per round trip is `spacing × order_size`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridConfig {
    /// Broker to trade against: always `"bullet"`.
    #[serde(default = "default_exchange")]
    pub exchange: String,

    /// Trading symbol (e.g. `"SOL-USD"`).
    pub symbol: String,

    /// The dollars the grid trades: its worst case (every level on the losing side filled) is
    /// held to them, and to `limits.max_notional_usd` when that is lower.
    #[serde(default = "default_amount")]
    pub amount_usd: Decimal,

    /// `neutral` starts flat; `long` / `short` open the position the far side's levels close.
    #[serde(default)]
    pub direction: Direction,

    /// How wide the derived range is (the lookback of the volatility that sets it).
    #[serde(default)]
    pub range: RangePreset,

    /// Lower bound of the grid (absolute price); derived when omitted.
    #[serde(default)]
    pub lower_price: Option<Decimal>,

    /// Upper bound of the grid (absolute price); derived when omitted.
    #[serde(default)]
    pub upper_price: Option<Decimal>,

    /// Levels across `[lower_price, upper_price]`, at least 2; derived when omitted.
    #[serde(default)]
    pub num_levels: Option<usize>,

    /// Buys at or below it, sells above. Defaults to the mid at start.
    #[serde(default)]
    pub anchor_price: Option<Decimal>,

    /// Order quantity per level; sized from the dollars when omitted.
    #[serde(default)]
    pub order_size: Option<Decimal>,

    /// `"post_only"` (default: a level that would cross goes at the touch), `"limit"`, `"market"`.
    #[serde(default = "default_order_type")]
    pub order_type: OrderType,

    /// The grid's own position cap (base units); the worst case must fit it.
    #[serde(default)]
    pub max_position: Option<Decimal>,

    /// The hard stop's distance past the losing edge, in range widths.
    #[serde(default = "default_stop_beyond")]
    pub stop_beyond: Decimal,

    /// Optional fee-floor guardrail: refuse to run when level spacing can't cover
    /// `min_spacing_fee_multiplier × round-trip maker fees`.
    #[serde(default)]
    pub fees: Option<GridFees>,

    /// Optional EMA-slope trend filter: past `pause_divergence_bps` the opening orders are
    /// cancelled and the closing ones stay.
    #[serde(default)]
    pub trend_filter: Option<TrendFilterConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridFees {
    pub maker_bps: Decimal,
    #[serde(default)]
    pub taker_bps: Decimal,
    #[serde(default = "default_min_spacing_fee_multiplier")]
    pub min_spacing_fee_multiplier: Decimal,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrendFilterConfig {
    #[serde(default = "default_fast_secs")]
    pub fast_secs: u64,
    #[serde(default = "default_slow_secs")]
    pub slow_secs: u64,
    pub pause_divergence_bps: Decimal,
}

/// The dollars a grid trades when the section names none.
pub const DEFAULT_AMOUNT_USD: Decimal = Decimal::from_parts(100, 0, 0, false, 0);
/// Half a range width past the losing edge.
pub const DEFAULT_STOP_BEYOND: Decimal = Decimal::from_parts(5, 0, 0, false, 1);
/// The range bounds an owner may give: positive, and no further from 1 than any market trades.
const RANGE_PRICE: Range = Range::new(Decimal::from_parts(1, 0, 0, false, 8), whole(1_000_000_000));
const MAX_FEE_MULTIPLIER: Decimal = whole(10_000);

fn default_order_type() -> OrderType {
    OrderType::PostOnly
}

fn default_exchange() -> String {
    "bullet".to_string()
}

fn default_amount() -> Decimal {
    DEFAULT_AMOUNT_USD
}

fn default_stop_beyond() -> Decimal {
    DEFAULT_STOP_BEYOND
}

fn default_min_spacing_fee_multiplier() -> Decimal {
    Decimal::from(3)
}

fn default_fast_secs() -> u64 {
    300
}

fn default_slow_secs() -> u64 {
    3600
}

impl GridConfig {
    /// Whether the start needs the market's price history: the range or the levels are derived.
    #[must_use]
    pub fn needs_history(&self) -> bool {
        self.lower_price.is_none() || self.upper_price.is_none() || self.num_levels.is_none()
    }

    /// The closed candles the derived range and step read, `(daily, hourly)`: the range preset's
    /// lookback of returns and the ATR window, each with the close before it.
    #[must_use]
    pub fn history_bars(&self) -> (u32, u32) {
        let atr = u32::try_from(ATR_PERIOD).unwrap_or(u32::MAX);
        (self.range.lookback_days().saturating_add(1), atr.saturating_add(1))
    }
}

impl ValidateConfig for GridConfig {
    /// Validate the config at startup. Returns a readable error for common
    /// misconfigurations rather than letting them surface as weird runtime
    /// behavior.
    fn validate(&self) -> Result<(), String> {
        if self.exchange != "bullet" {
            return Err(format!(
                "exchange {:?}: the grid trades on Bullet only, so exchange must be \"bullet\"",
                self.exchange
            ));
        }
        if self.amount_usd <= Decimal::ZERO {
            return Err("amount_usd must be > 0".to_string());
        }
        if self.num_levels.is_some_and(|n| n < 2) {
            return Err("num_levels must be ≥ 2".to_string());
        }
        // Bullet's 100 open orders a market.
        if self.num_levels.is_some_and(|n| n > 100) {
            return Err("num_levels must be ≤ 100".to_string());
        }
        if let (Some(lower), Some(upper)) = (self.lower_price, self.upper_price)
            && lower >= upper
        {
            return Err(format!("lower_price ({lower}) must be < upper_price ({upper})"));
        }
        // A price on any market, as `anchor_price`: past it the grid's arithmetic could overflow.
        for (name, price) in [("lower_price", self.lower_price), ("upper_price", self.upper_price)]
        {
            if price.is_some_and(|p| !RANGE_PRICE.contains(p)) {
                return Err(format!("{name} must be within [0.00000001, 1000000000]"));
            }
        }
        if self.order_size.is_some_and(|q| q <= Decimal::ZERO) {
            return Err("order_size must be > 0".to_string());
        }
        if self.max_position.is_some_and(|q| q <= Decimal::ZERO) {
            return Err("max_position must be > 0".to_string());
        }
        if self.stop_beyond <= Decimal::ZERO {
            return Err("stop_beyond must be > 0: the hard stop is always on".to_string());
        }
        if self.stop_beyond < Decimal::new(5, 2) || self.stop_beyond > Decimal::TWO {
            return Err("stop_beyond must be within [0.05, 2] range widths".to_string());
        }
        if let Some(fees) = &self.fees {
            for (name, bps) in [("maker_bps", fees.maker_bps), ("taker_bps", fees.taker_bps)] {
                if bps < Decimal::ZERO || bps > Decimal::from(1_000) {
                    return Err(format!("fees.{name} must be within [0, 1000]"));
                }
            }
            // Past it no grid starts at a fee of 1 bps or more (the floor passes 10000 bps); far past
            // it, the floor's product overflows.
            if fees.min_spacing_fee_multiplier > MAX_FEE_MULTIPLIER {
                return Err(format!(
                    "fees.min_spacing_fee_multiplier must be at most {MAX_FEE_MULTIPLIER}"
                ));
            }
        }
        if self
            .anchor_price
            .is_some_and(|a| a < Decimal::new(1, 8) || a > Decimal::from(1_000_000_000))
        {
            return Err("anchor_price must be within [0.00000001, 1000000000]".to_string());
        }
        if let (Some(anchor), Some(lower), Some(upper)) =
            (self.anchor_price, self.lower_price, self.upper_price)
            && (anchor < lower || anchor > upper)
        {
            return Err(format!("anchor_price ({anchor}) must be within [{lower}, {upper}]"));
        }
        Ok(())
    }
}

const ORDER_TYPES: &[&str] = &["limit", "post_only", "market"];

/// Every `[strategies.grid]` setting.
#[must_use]
#[expect(clippy::too_many_lines, reason = "one declaration per setting")]
pub fn settings() -> Vec<Setting> {
    let any = Range::RUNTIME;
    // A guard's guarded range ends at its default on the loose side: the agent tightens within
    // it, and only the owner signs past it (security agent-surface-M3).
    let at_least = |n| Range { min: Some(whole(n)), max: None };
    vec![
        Setting::of(
            ParamSpec::text("exchange", "bullet", "The registered exchange the grid trades on.")
                .tier(Tier::Structural),
        ),
        Setting::of(ParamSpec::text("symbol", "SOL-USD", "Trading symbol.").tier(Tier::Structural))
            .required(),
        Setting::of(
            ParamSpec::decimal(
                "amount_usd",
                DEFAULT_AMOUNT_USD,
                Range::whole(10, 5_000),
                any,
                "Dollars across all its levels: the most it holds with every losing-side level \
                 filled.",
            )
            .tier(Tier::Limit)
            .loosening(Loosening::Up),
        ),
        Setting::of(
            ParamSpec::choice(
                "direction",
                Direction::NAMES,
                0,
                "neutral starts flat; long buys the levels above the price at start, short sells \
                 the ones below.",
            )
            .tier(Tier::Structural),
        ),
        Setting::of(
            // A wider range puts the hard stop further out, as a wider lower_price or upper_price
            // does: widening it loosens, and the guarded range ends at the default.
            ParamSpec {
                guarded: Range::whole(0, 1),
                ..ParamSpec::choice(
                    "range",
                    RangePreset::NAMES,
                    1,
                    "How wide the range is: 3 standard deviations of daily moves over 7 (tight), \
                     30 (normal) or 90 (wide) days.",
                )
                .tier(Tier::Structural)
                .loosening(Loosening::Up)
            },
        ),
        Setting::of(
            ParamSpec::decimal(
                "lower_price",
                whole(1),
                RANGE_PRICE,
                RANGE_PRICE,
                "Lower bound of the grid; derived from the price history when omitted.",
            )
            .tier(Tier::Limit)
            .loosening(Loosening::Down),
        ),
        Setting::of(
            ParamSpec::decimal(
                "upper_price",
                whole(2),
                RANGE_PRICE,
                RANGE_PRICE,
                "Upper bound of the grid; derived when omitted.",
            )
            .tier(Tier::Limit)
            .loosening(Loosening::Up),
        ),
        Setting::of(
            ParamSpec::int(
                "num_levels",
                11,
                Range::whole(2, 50),
                Range::whole(2, 100),
                "Levels across [lower_price, upper_price]; derived when omitted.",
            )
            .tier(Tier::Limit)
            .loosening(Loosening::Up),
        ),
        Setting::of(
            ParamSpec::decimal(
                "anchor_price",
                whole(1),
                Range::new(Decimal::new(1, 8), whole(1_000_000_000)),
                Range::new(Decimal::new(1, 8), whole(1_000_000_000)),
                "Buys below, sells above; omit for the mid at start.",
            )
            .tier(Tier::Structural),
        ),
        Setting::of(
            ParamSpec::decimal(
                "order_size",
                whole(1),
                any,
                any,
                "Order quantity per level; sized from amount_usd when omitted.",
            )
            .tier(Tier::Limit)
            .loosening(Loosening::Up),
        ),
        Setting::of(
            ParamSpec::choice("order_type", ORDER_TYPES, 1, "Order type for grid orders.")
                .tier(Tier::Structural),
        ),
        Setting::of(
            ParamSpec::decimal(
                "max_position",
                whole(1),
                any,
                any,
                "The grid's own position cap (base units); the worst case must fit it.",
            )
            .tier(Tier::Limit)
            .loosening(Loosening::Up),
        ),
        Setting::of(
            ParamSpec::decimal(
                "stop_beyond",
                DEFAULT_STOP_BEYOND,
                Range::new(Decimal::new(1, 1), Decimal::ONE),
                Range::new(Decimal::new(5, 2), Decimal::TWO),
                "The hard stop's distance past the losing edge, in range widths: it closes \
                 everything and stops there.",
            )
            .tier(Tier::Limit)
            .loosening(Loosening::Up),
        ),
        Setting::under(
            "fees",
            // The fee floor is built from it: lowering a fee the owner set loosens the floor.
            ParamSpec::decimal(
                "maker_bps",
                whole(0),
                Range::new(whole(0), whole(1000)),
                Range::new(whole(0), whole(1000)),
                "Maker fee (bps); the fee floor is built from it, so lowering it loosens the floor.",
            )
            .tier(Tier::Structural)
            .loosening(Loosening::Down),
        ),
        Setting::under(
            "fees",
            ParamSpec::decimal(
                "taker_bps",
                whole(0),
                Range::whole(0, 1_000),
                Range::whole(0, 1_000),
                "Taker fee (bps).",
            )
                .tier(Tier::Structural),
        ),
        Setting::under(
            "fees",
            ParamSpec::decimal(
                "min_spacing_fee_multiplier",
                whole(3),
                Range { min: Some(whole(3)), max: Some(MAX_FEE_MULTIPLIER) },
                Range { min: None, max: Some(MAX_FEE_MULTIPLIER) },
                "Level spacing must cover this many round-trip maker fees.",
            )
            .tier(Tier::Structural)
            .loosening(Loosening::Down),
        ),
        Setting::under(
            "trend_filter",
            ParamSpec::int("fast_secs", 300, Range::whole(1, 300), any, "Fast EMA window (s).")
                .tier(Tier::Structural)
                .loosening(Loosening::Up),
        ),
        Setting::under(
            "trend_filter",
            ParamSpec::int("slow_secs", 3600, at_least(3600), any, "Slow EMA window (s).")
                .tier(Tier::Structural)
                .loosening(Loosening::Down),
        ),
        Setting::under(
            "trend_filter",
            ParamSpec::decimal(
                "pause_divergence_bps",
                whole(50),
                Range::whole(0, 50),
                any,
                "Past this fast/slow EMA divergence (bps) the opening orders are cancelled.",
            )
            .tier(Tier::Structural)
            .loosening(Loosening::Up),
        ),
    ]
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
