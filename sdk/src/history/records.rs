//! The history data a strategy core reads and asks the host to record: a fill with its quality
//! measures ([`FillRecord`]), the window measures ([`WindowMetrics`]), what a restart goes on from
//! ([`Restored`], [`Fees`]), and the records a core hands the host ([`HistoryEvent`]). A core
//! never holds the host's store: it reads through `dyn HistoryReads` and the host applies its
//! events in order. The store itself is the host's.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::types::{Side, Trade};

/// A fill with its quality measures.
///
/// `spread_captured_bps`: bps earned versus the mid at fill; positive means
/// we bought below / sold above mid.
///
/// Markout convention: a **positive markout means the fill was good for us**
/// (price moved our way: bought then mid rose, or sold then mid fell),
/// `side_sign * (mid_later - mid_at_fill) / mid_at_fill` in bps, with
/// `side_sign = +1` for a buy and `-1` for a sell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FillRecord {
    pub id: u64,
    pub ts_ms: u64,
    pub side: Side,
    pub price: Decimal,
    pub quantity: Decimal,
    pub mid_at_fill: Decimal,
    pub spread_captured_bps: f64,
    #[serde(default)]
    pub markout_5s_bps: Option<f64>,
    #[serde(default)]
    pub markout_60s_bps: Option<f64>,
    /// A would-be fill from paper trading (dry run), not a real execution.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paper: bool,
    /// Took liquidity (a close IOC), so it pays the taker fee.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub taker: bool,
    /// The build version (`N` of `vN`) that made the fill; `None` for an
    /// unversioned build and for fills recorded before versions existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    /// The live config version (`bb-config`) in force at the fill; `None`
    /// without one and for fills recorded before config versions existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_version: Option<u64>,
    /// The venue's id of the fill (`Trade::trade_id`), which a backfill of missed fills dedups on;
    /// `None` without one and for fills recorded before it was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trade_id: Option<String>,
}

/// How a fill is labelled in the history (see [`FillRecord`]'s fields).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FillKind {
    /// A paper-trading (dry run) would-be fill.
    pub paper: bool,
    /// Took liquidity (a close IOC).
    pub taker: bool,
}

/// Quality of quoting over a window, for judging a parameter change.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowMetrics {
    pub fills: u64,
    pub fill_rate_per_hour: f64,
    /// Mean over the window's fills.
    pub spread_captured_bps: Option<f64>,
    /// Mean over the window's fills that have one.
    pub markout_60s_bps: Option<f64>,
    /// Day P&L at the window's last snapshot.
    pub day_pnl: Option<f64>,
}

/// The venue's fees, in bps of a fill's notional: a paper fill pays its own side's, a live one the
/// higher of the two (the venue does not say which it was), as the host counts them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fees {
    pub maker_bps: Option<Decimal>,
    pub taker_bps: Option<Decimal>,
}

impl Fees {
    #[must_use]
    pub fn of(&self, fill: &FillRecord) -> Decimal {
        self.of_kind(FillKind { paper: fill.paper, taker: fill.taker })
    }

    #[must_use]
    pub fn of_kind(&self, kind: FillKind) -> Decimal {
        let (maker, taker) = (self.maker_bps, self.taker_bps);
        let bps = match (kind.paper, kind.taker) {
            (true, true) => taker,
            (true, false) => maker,
            (false, _) => maker.max(taker),
        };
        bps.unwrap_or_default()
    }
}

/// What the history says a build of one mode had at `now_ms`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Restored {
    /// Realized P&L over the mode's fills, before fees (the host's inventory counts it so).
    pub realized_pnl: Decimal,
    /// The mode's fills in the history.
    pub fills: u64,
    /// Of those, the fills since 00:00 UTC.
    pub fills_today: u64,
    /// Today's P&L from trading (fees in, never a deposit): `None` with neither a fill nor a point
    /// of the mode today.
    pub day: Option<Decimal>,
}

/// What a strategy core may read back from the history: fills and window measures. No write.
pub trait HistoryReads: Send + Sync {
    /// Fills in `[from_ms, to_ms]` with markouts merged in, oldest first.
    fn fills(&self, from_ms: u64, to_ms: u64) -> Vec<FillRecord>;

    /// How quoting went over `[from_ms, to_ms]`.
    fn window_metrics(&self, from_ms: u64, to_ms: u64) -> WindowMetrics;

    /// What a build of the mode `paper` goes on from at `now_ms` (the host store's `restored`);
    /// nothing for a history that keeps none.
    fn restored(&self, _paper: bool, _now_ms: u64, _fees: Fees) -> Restored {
        Restored::default()
    }
}

/// A record a strategy core asks the host to make, in the order it made it.
#[derive(Debug, Clone)]
pub enum HistoryEvent {
    /// The host store's `record`.
    Fill { trade: Trade, mid_at_fill: Decimal, now_ms: u64, kind: FillKind },
    /// Its `observe_mid`.
    Mid { mid: Decimal, now_ms: u64 },
    /// Its `record_point`.
    Point { day_pnl_net: Option<Decimal>, net_position: Decimal, now_ms: u64 },
}

#[cfg(test)]
#[path = "records_tests.rs"]
mod tests;
