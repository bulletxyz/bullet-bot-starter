//! The grid the strategy runs, resolved once at start from its config, the market (mid, tick,
//! lot), the price history the build read, and the hard limits: the range, the levels and their
//! sides, the size per level, the starting position, the hard stops and the worst case. A grid
//! whose worst case does not fit the dollars and the hard limits is refused here.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal prices, sizes and notionals sit far below 7.9e28; level counts are tiny and \
              checked >= MIN_LEVELS before any subtraction"
)]

use bb_strategy_sdk::Side;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::config::GridConfig;
use crate::derive::Candle;
use crate::derive::{self, Direction, MIN_LEVELS};

/// The price history the build read for the geometry (Binance's candles), oldest first.
#[derive(Debug, Clone, Default)]
pub struct History {
    /// Daily candles over the range preset's lookback.
    pub daily: Vec<Candle>,
    /// Hourly candles for the ATR.
    pub hourly: Vec<Candle>,
}

/// The config's hard limits (`limits.*`), which the order path enforces too: the grid refuses a
/// worst case past them before it places anything.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HardLimits {
    pub max_position: Option<Decimal>,
    pub max_notional_usd: Option<Decimal>,
}

/// What the start sees of the market.
#[derive(Debug, Clone, Copy)]
pub struct Market {
    pub mid: Decimal,
    pub tick: Option<Decimal>,
    pub lot: Option<Decimal>,
}

/// The resolved grid.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    pub lower: Decimal,
    pub upper: Decimal,
    pub levels: usize,
    pub order_size: Decimal,
    /// Buys at or below it, sells above, the level within half a spacing of it empty.
    pub anchor: Decimal,
    pub direction: Direction,
    /// The starting position's IOC (long: a buy of the levels above the anchor; short: a sell of
    /// those below).
    pub entry: Option<(Side, Decimal)>,
    /// Flatten and stop at or below it (long, and neutral's long side).
    pub stop_below: Option<Decimal>,
    /// Flatten and stop at or above it (short, and neutral's short side).
    pub stop_above: Option<Decimal>,
    /// The position with every risk-side level filled, in coins.
    pub worst_coins: Decimal,
    /// What that position cost, in USD.
    pub worst_notional: Decimal,
    /// The dollars it was sized from (the config's, or the notional limit when lower).
    pub dollars: Decimal,
}

impl Plan {
    #[must_use]
    pub fn spacing(&self) -> Decimal {
        (self.upper - self.lower) / Decimal::from(self.levels as u64 - 1)
    }

    /// Level `i`'s price.
    #[must_use]
    pub fn price(&self, i: usize, tick: Option<Decimal>) -> Decimal {
        let p = self.lower + self.spacing() * Decimal::from(i as u64);
        match tick {
            Some(t) if t > Decimal::ZERO => (p / t).round() * t,
            _ => p,
        }
    }

    /// Each level's price and side: buys at or below the anchor, sells above, and `None` for the
    /// one level nearest the anchor when it is within half a spacing (an anchor exactly halfway
    /// between two levels empties neither).
    #[must_use]
    pub fn sides(&self, tick: Option<Decimal>) -> Vec<(Decimal, Option<Side>)> {
        let prices: Vec<Decimal> = (0..self.levels).map(|i| self.price(i, tick)).collect();
        let empty = prices
            .iter()
            .enumerate()
            .map(|(i, p)| (i, (*p - self.anchor).abs()))
            .min_by_key(|(_, gap)| *gap)
            .filter(|(_, gap)| *gap * Decimal::TWO < self.spacing())
            .map(|(i, _)| i);
        prices
            .into_iter()
            .enumerate()
            .map(|(i, p)| {
                let side = if Some(i) == empty {
                    None
                } else if p <= self.anchor {
                    Some(Side::Buy)
                } else {
                    Some(Side::Sell)
                };
                (p, side)
            })
            .collect()
    }
}

/// `(buys, sells)`: the prices of each side's levels for `levels` across the range.
fn layout(
    (lower, upper, levels): (Decimal, Decimal, usize),
    anchor: Decimal,
    tick: Option<Decimal>,
) -> (Vec<Decimal>, Vec<Decimal>) {
    let probe = Plan {
        lower,
        upper,
        levels,
        order_size: Decimal::ZERO,
        anchor,
        direction: Direction::Neutral,
        entry: None,
        stop_below: None,
        stop_above: None,
        worst_coins: Decimal::ZERO,
        worst_notional: Decimal::ZERO,
        dollars: Decimal::ZERO,
    };
    let (mut buys, mut sells) = (Vec::new(), Vec::new());
    for (p, side) in probe.sides(tick) {
        match side {
            Some(Side::Buy) => buys.push(p),
            Some(Side::Sell) => sells.push(p),
            None => {}
        }
    }
    (buys, sells)
}

/// The worst case per coin of order size: `(coins, usd)` with every risk-side level filled (and
/// the starting position bought or sold at `mid`).
fn worst_per_unit(
    direction: Direction,
    (buys, sells): (&[Decimal], &[Decimal]),
    mid: Decimal,
) -> (Decimal, Decimal) {
    let n = |v: &[Decimal]| Decimal::from(v.len() as u64);
    let sum = |v: &[Decimal]| v.iter().copied().sum::<Decimal>();
    match direction {
        Direction::Neutral => {
            let (long, short) = ((n(buys), sum(buys)), (n(sells), sum(sells)));
            (long.0.max(short.0), long.1.max(short.1))
        }
        Direction::Long => (n(buys) + n(sells), sum(buys) + n(sells) * mid),
        Direction::Short => (n(buys) + n(sells), sum(sells) + n(buys) * mid),
    }
}

fn floor_to(v: Decimal, lot: Option<Decimal>) -> Decimal {
    match lot {
        Some(l) if l > Decimal::ZERO => (v / l).floor() * l,
        _ => v,
    }
}

fn money(v: Decimal) -> String {
    format!("${}", v.round_dp(2).normalize())
}

/// Resolve the grid for `cfg` at `market`.
///
/// # Errors
/// What the owner must change, in plain words: no price history for a range it must derive, a
/// mid outside the range, dollars too few for its levels, or a worst case past the dollars or a
/// hard limit.
#[expect(clippy::too_many_lines, reason = "one resolution, in order")]
pub fn resolve(
    cfg: &GridConfig,
    market: Market,
    history: Option<&History>,
    limits: HardLimits,
) -> Result<Plan, String> {
    let Market { mid, tick, lot } = market;
    let sym = cfg.symbol();
    // The config's dollars, or the notional limit when lower (the real-money deposit sets it).
    let dollars = limits.max_notional_usd.map_or(cfg.amount_usd, |n| n.min(cfg.amount_usd));
    let (lower, upper, mut levels) = match (cfg.lower_price, cfg.upper_price, cfg.num_levels) {
        (Some(l), Some(u), Some(n)) => (l, u, n),
        (given_lower, given_upper, given_levels) => {
            let (Some(tick), Some(lot)) = (tick, lot) else {
                return Err(format!(
                    "The venue gave no tick or lot size for {sym}, so the grid can't be laid out"
                ));
            };
            let h = history.ok_or_else(|| {
                format!("The grid needs {sym}'s price history to set its range and step")
            })?;
            let lookback = cfg.range.lookback_days() as usize;
            let daily = &h.daily[h.daily.len().saturating_sub(lookback + 1)..];
            let sigma = derive::daily_sigma(daily).ok_or_else(|| {
                format!("Not enough daily prices for {sym} to measure how much it moves")
            })?;
            let atr = derive::atr(&h.hourly).ok_or_else(|| {
                format!("Not enough hourly prices for {sym} to set the grid's step")
            })?;
            let (lo, hi) = derive::range(mid, sigma, cfg.range, tick);
            let (l, u) = (given_lower.unwrap_or(lo), given_upper.unwrap_or(hi));
            let n = given_levels.unwrap_or_else(|| {
                let cap = derive::levels_for_dollars(dollars, lot, mid);
                derive::levels(l, u, derive::step(atr, tick, mid), cap)
            });
            (l, u, n)
        }
    };
    if lower <= Decimal::ZERO || lower >= upper {
        return Err(format!("The grid's range {lower} to {upper} is empty"));
    }
    if mid < lower || mid > upper {
        return Err(format!(
            "{sym} is at {mid}, outside the grid's range {lower} to {upper}: set a range around \
             the price (or leave it out to derive one)"
        ));
    }
    let anchor = cfg.anchor_price.unwrap_or(mid);
    let fixed_levels = cfg.num_levels.is_some();
    let min_levels = if fixed_levels { 2 } else { MIN_LEVELS };
    let (buys, sells, order_size) = loop {
        if levels < min_levels {
            return Err(if fixed_levels {
                format!("The grid needs at least {min_levels} levels")
            } else {
                format!(
                    "{} is too small for a grid on {sym}: each of its {MIN_LEVELS} levels needs \
                     at least one order of {}",
                    money(dollars),
                    lot.map_or_else(|| "the venue's least size".into(), |l| format!("{l}"))
                )
            });
        }
        let (buys, sells) = layout((lower, upper, levels), anchor, tick);
        let size = cfg.order_size.unwrap_or_else(|| {
            let (_, usd) = worst_per_unit(cfg.direction, (&buys, &sells), mid);
            if usd <= Decimal::ZERO { Decimal::ZERO } else { floor_to(dollars / usd, lot) }
        });
        let too_small = size <= Decimal::ZERO || lot.is_some_and(|l| size < l);
        if too_small && cfg.order_size.is_none() && !fixed_levels {
            levels -= 1;
            continue;
        }
        if too_small {
            return Err(format!(
                "The grid's order size {size} is below {sym}'s least order {}",
                lot.unwrap_or_default()
            ));
        }
        break (buys, sells, size);
    };
    if buys.is_empty() && sells.is_empty() {
        return Err("The grid has no level to place an order at".into());
    }
    let entry = match cfg.direction {
        Direction::Neutral => None,
        Direction::Long => Some((Side::Buy, order_size * Decimal::from(sells.len() as u64))),
        Direction::Short => Some((Side::Sell, order_size * Decimal::from(buys.len() as u64))),
    }
    .filter(|(_, q)| *q > Decimal::ZERO);
    let (coins, usd) = worst_per_unit(cfg.direction, (&buys, &sells), mid);
    let (worst_coins, worst_notional) = (coins * order_size, usd * order_size);
    let width = upper - lower;
    let beyond = width * cfg.stop_beyond;
    let (stop_below, stop_above) = match cfg.direction {
        Direction::Neutral => (Some(lower - beyond), Some(upper + beyond)),
        Direction::Long => (Some(lower - beyond), None),
        Direction::Short => (None, Some(upper + beyond)),
    };
    let plan = Plan {
        lower,
        upper,
        levels,
        order_size,
        anchor,
        direction: cfg.direction,
        entry,
        stop_below,
        stop_above,
        worst_coins,
        worst_notional,
        dollars,
    };
    check_worst(&plan, cfg, limits)?;
    Ok(plan)
}

/// Rule 1: the position with every risk-side level filled must fit the dollars, the grid's own
/// cap and the hard limits.
fn check_worst(plan: &Plan, cfg: &GridConfig, limits: HardLimits) -> Result<(), String> {
    let (coins, usd) = (plan.worst_coins, plan.worst_notional);
    let what = format!(
        "With every level on its losing side filled the grid would hold {coins} {} ({})",
        cfg.symbol(),
        money(usd)
    );
    if usd > cfg.amount_usd {
        return Err(format!(
            "{what}, more than its {}: fewer levels or a smaller size",
            money(cfg.amount_usd)
        ));
    }
    if let Some(n) = limits.max_notional_usd
        && usd > n
    {
        return Err(format!("{what}, past the notional limit {}", money(n)));
    }
    for cap in [cfg.max_position, limits.max_position].into_iter().flatten() {
        if coins > cap {
            return Err(format!("{what}, past the position limit {cap}"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
