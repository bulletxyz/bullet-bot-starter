//! The daily loss stop's state: what survives a restart ([`PersistedStops`]), the day's P&L
//! ([`DayPnl`]), the account's value at the day's start ([`DayOpen`]) and why quoting or ordering stopped ([`StopReason`]). Shared by the market maker's
//! guards and the self-trading strategies' loss stop; where the stops are kept is the host's.

use chrono::{DateTime, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Why quoting is not allowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StopReason {
    /// Day P&L (net of fees) hit the daily loss limit. Clears at 00:00 UTC.
    DailyLoss { day: NaiveDate },
    /// No book update for `stale_secs`. Clears on the next book update. `market`: the one with no
    /// fresh book, when a leg's stop names it (left out otherwise, as before).
    StalePrice {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        market: Option<String>,
    },
    /// Mid moved more than `fast_move_bps` within `fast_window_secs` (`GuardParams` in bb-mm).
    FastMarket { until_ms: u64 },
    /// Reject breaker tripped. Cleared only by the user resuming.
    Rejects,
    /// Paused by the user.
    Paused,
    /// The venue left a run of orders unanswered (a timeout, a 5xx): new orders are stopped until
    /// a read accounts for them, which lifts it by itself.
    VenueSilent { orders: u32 },
    /// The key's delegation moved on the venue: nothing goes out until a restart.
    DelegationMoved,
    /// The saved stops couldn't be read, so new orders are refused until the file is fixed.
    StopsUnreadable,
    /// The venue's position differs from the fills and the gap is not confirmed yet: no new orders.
    PositionUnconfirmed,
    /// The account's value can't be read, so the daily loss stop is blind: no new orders until it
    /// can.
    EquityUnread,
    /// The day's loss can't be saved, so a restart would forget it: no new orders until it can.
    DayStateUnsaved,
    /// Live, the account it trades holds no money: no new orders until it does.
    NoMoney,
}

pub const KEY_DAILY_LOSS: &str = "daily_loss";

/// The UTC calendar day of a Unix-ms timestamp.
#[must_use]
pub fn utc_day(now_ms: u64) -> NaiveDate {
    // Unrepresentable timestamps map to MIN: never after a tripped stop's day,
    // so the stop stays active.
    i64::try_from(now_ms)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
        .map_or(NaiveDate::MIN, |dt| dt.date_naive())
}

/// A market a safety hold kept, with the venue its leg is on. An entry saved before the venue was
/// recorded is a bare symbol: it stands for every venue the instance declares, is released only
/// when all of them read flat, and is saved back bare.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HeldMarket {
    /// `{"venue": "<venue name>", "symbol": "BTC-USD"}`.
    On { venue: String, symbol: String },
    /// `"BTC-USD"`: on every venue the instance declares.
    Bare(String),
}

impl HeldMarket {
    #[must_use]
    pub fn on(venue: impl Into<String>, symbol: impl Into<String>) -> Self {
        Self::On { venue: venue.into(), symbol: symbol.into() }
    }

    #[must_use]
    pub fn symbol(&self) -> &str {
        match self {
            Self::On { symbol, .. } | Self::Bare(symbol) => symbol,
        }
    }

    /// The venue its leg is on (`None`: every venue the instance declares).
    #[must_use]
    pub fn venue(&self) -> Option<&str> {
        match self {
            Self::On { venue, .. } => Some(venue),
            Self::Bare(_) => None,
        }
    }
}

/// The stops that survive a restart (`stops.json`; the format is frozen).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "a saved document: each flag is its own stop, written only when set"
)]
pub struct PersistedStops {
    #[serde(default)]
    pub daily_loss_day: Option<NaiveDate>,
    #[serde(default)]
    pub paused: bool,
    /// The pause is a safety hold's (a daily loss trip, an account halt, the kill switch): a
    /// restart held on it closes what is left. Written only when set (the file's format is
    /// unchanged).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paused_for_safety: bool,
    /// The markets a universe strategy declared when its safety hold was saved, each with the
    /// venue its leg is on: a held start closes these as well as the ones it finds then, so a
    /// market that has since left its universe is never read as flat. Written only when set (the
    /// file's format is unchanged).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub held_markets: Vec<HeldMarket>,
    /// The instance's last run that traded was paper (its own mode, or held there): its next live
    /// start first reads its markets for the real legs a live run left there. Written only when
    /// set (the file's format is unchanged).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ran_paper: bool,
    /// The markets every paper run declared since the last live start, each on its venue: that
    /// live start reads them all for the real legs a live run left there (a refused or failed
    /// build, a run between and a restart all keep them). Written only when set (the file's format
    /// is unchanged).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paper_markets: Vec<HeldMarket>,
    /// UTC day the carry belongs to.
    #[serde(default)]
    pub day_pnl_day: Option<NaiveDate>,
    /// Day P&L (net of fees) at the last write, carried into a restart on
    /// the same UTC day.
    #[serde(default)]
    pub day_pnl_carry: Option<Decimal>,
    /// Written only by a migration of an older version's `day_state.json`: the account's value at
    /// the first read of `day_pnl_day`, which the day's P&L is counted from once; the next save
    /// writes the carry instead. Not written otherwise (the file's format is unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day_baseline: Option<Decimal>,
    /// The account's value at the last read of `day_pnl_day` (the self-trading strategies' loss
    /// stop): a restart the same day counts the move from it to its first read, what a position
    /// kept over the restart made while the bot was down. Written only when set (the file's format
    /// is unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day_equity: Option<Decimal>,
    /// The accounts `day_equity` is the value of together (each leg's venue): a start over other
    /// accounts (a strategy switched the same day) reads another sum, so it counts no gap.
    /// Written only when set (the file's format is unchanged).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub day_equity_accounts: Vec<String>,
    /// The market, the position and the mid at the last P&L update of `day_pnl_day` (the market
    /// maker's guards): a restart the same day on that market, holding that position, counts what
    /// it made between that mid and its first one. Written only when set (the file's format is
    /// unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day_mark: Option<DayMark>,
    /// Today's P&L from trading alone (fills and the position marked, fees estimated; no deposit
    /// or withdrawal) at the last save of `day_pnl_day` (the self-trading strategies' loss stop):
    /// a restart the same day goes on from it. Written only when set (the file's format is
    /// unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day_trading_pnl: Option<Decimal>,
    /// The account's value at the start of its UTC day ([`DayOpen`]). Written only when set (the
    /// file's format is unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day_start: Option<DayStart>,
    /// The market maker's reject breaker tripped; only the owner's resume clears it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rejects_tripped: bool,
    /// The strategy host's cancel cap tripped; only the owner's resume clears it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel_rate_tripped: bool,
    /// The stop the last live entry carried, placed again by a start over that position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_stop: Option<EntryStop>,
}

impl PersistedStops {
    /// `self` with the strategy host's own fields as `saved` has them, so other writers never
    /// clear them.
    #[must_use]
    pub fn keeping_host(self, saved: &Self) -> Self {
        Self {
            cancel_rate_tripped: saved.cancel_rate_tripped,
            entry_stop: saved.entry_stop.clone(),
            ..self
        }
    }
}

/// [`PersistedStops::entry_stop`]: the side is the position's it guards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryStop {
    pub symbol: String,
    pub side: crate::types::Side,
    pub price: Decimal,
}

/// A position on a market and the mid it was valued at ([`PersistedStops::day_mark`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayMark {
    pub symbol: String,
    pub position: Decimal,
    pub mid: Decimal,
}

/// The account's value at the first read of a UTC day ([`PersistedStops::day_start`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayStart {
    pub day: NaiveDate,
    pub value: Decimal,
}

/// The account's value at the start of the UTC day, what the daily loss stop is sized from: the
/// first read of each day (or, the first day, at go-live: a [`DayOpen::default`]) records it and
/// nothing moves it that day, neither the day's P&L nor a deposit. A restart goes on from the
/// saved one ([`Self::load`]).
#[derive(Debug, Clone, Default)]
pub struct DayOpen(Open);

#[derive(Debug, Clone, Copy, Default)]
enum Open {
    /// No day's start yet: the next read records it.
    #[default]
    Unread,
    Known(DayStart),
    /// An older version read the account on this day without saving its start: the value now
    /// may count a deposit made since, so the day has none, and the next day's first read records
    /// one.
    Unknown(NaiveDate),
}

impl DayOpen {
    /// From the saved stops. A file without a start that an earlier process read today (an older
    /// version's, or one a writer without it saved) leaves today's start unknown.
    #[must_use]
    pub fn load(saved: Option<&PersistedStops>) -> Self {
        Self(match saved {
            Some(PersistedStops { day_start: Some(start), .. }) => Open::Known(*start),
            Some(PersistedStops { day_pnl_day: Some(day), .. }) => Open::Unknown(*day),
            _ => Open::Unread,
        })
    }

    /// A read of the account's `value` at `now_ms`: the day's start value, recorded if this is the
    /// day's first read. `None` while today's start is unknown: no new orders until the next day.
    /// The day never goes backwards: a clock stepping back keeps the day's start.
    pub fn read(&mut self, value: Decimal, now_ms: u64) -> Option<Decimal> {
        let now = utc_day(now_ms);
        match self.0 {
            Open::Known(start) if now <= start.day => Some(start.value),
            Open::Unknown(day) if now <= day => None,
            _ => {
                self.0 = Open::Known(DayStart { day: now, value });
                Some(value)
            }
        }
    }

    /// What to save in [`PersistedStops::day_start`]. An unknown start saves none: the saved
    /// `day_pnl_day` of that day keeps it unknown over a restart.
    #[must_use]
    pub fn to_persist(&self) -> Option<DayStart> {
        match self.0 {
            Open::Known(start) => Some(start),
            Open::Unread | Open::Unknown(_) => None,
        }
    }
}

/// Day P&L: the change in total net P&L since the first observation of the
/// current UTC day, plus any carry from an earlier process on the same day.
#[derive(Debug, Clone, Default)]
pub struct DayPnl {
    baseline: Option<(NaiveDate, Decimal)>,
    carry: Option<(NaiveDate, Decimal)>,
    last: Option<(NaiveDate, Decimal)>,
}

impl DayPnl {
    /// Start with P&L already accumulated on `day` by a previous process.
    #[must_use]
    pub fn with_carry(day: NaiveDate, carry: Decimal) -> Self {
        Self { carry: Some((day, carry)), ..Self::default() }
    }

    /// Count the day's P&L from `baseline`, the account's value at the day's first read (an earlier
    /// version's saved day).
    #[must_use]
    pub fn with_baseline(day: NaiveDate, baseline: Decimal) -> Self {
        Self { baseline: Some((day, baseline)), ..Self::default() }
    }

    /// Feed the running total (realised + unrealised − fees); returns day P&L, saturating past a
    /// `Decimal` (a loss stays a loss).
    pub fn update(&mut self, total_net: Decimal, now_ms: u64) -> Decimal {
        let today = self.today(now_ms);
        let base = match self.baseline {
            Some((day, base)) if day == today => base,
            _ => {
                self.baseline = Some((today, total_net));
                total_net
            }
        };
        let carry = match self.carry {
            Some((day, c)) if day == today => c,
            _ => {
                self.carry = None;
                Decimal::ZERO
            }
        };
        let pnl = total_net.saturating_sub(base).saturating_add(carry);
        self.last = Some((today, pnl));
        pnl
    }

    /// The day an update at `now_ms` counts toward. The day never goes backwards: a clock stepping
    /// back keeps the day's baseline.
    fn today(&self, now_ms: u64) -> NaiveDate {
        let latest =
            [self.baseline, self.last, self.carry].into_iter().flatten().map(|(day, _)| day).max();
        utc_day(now_ms).max(latest.unwrap_or(NaiveDate::MIN))
    }

    /// Money moved in (`net` above zero) or out of the account today, which is no P&L: the day's
    /// first value moves with it.
    pub fn add_moved(&mut self, net: Decimal, now_ms: u64) {
        let today = self.today(now_ms);
        if let Some((day, base)) = &mut self.baseline
            && *day == today
        {
            *base = base.saturating_add(net);
        }
    }

    /// Add `amount` to the carry an earlier process saved, only while it is the day an update at
    /// `now_ms` counts toward (what moved while no process read the day); saturates.
    pub fn add_to_carry(&mut self, amount: Decimal, now_ms: u64) {
        let today = self.today(now_ms);
        if let Some((day, carry)) = &mut self.carry
            && *day == today
        {
            *carry = carry.saturating_add(amount);
        }
    }

    /// Take over `other`'s baseline (a guard built mid-run counts the day from it).
    pub fn start_from(&mut self, other: &Self) {
        self.baseline = other.baseline;
    }

    /// A baseline loaded from a migrated day and not yet read against (no read, no carry): what a
    /// save before the first read must write back.
    #[must_use]
    pub fn pending_baseline(&self) -> Option<(NaiveDate, Decimal)> {
        if self.last.is_some() || self.carry.is_some() {
            return None;
        }
        self.baseline
    }

    /// The day and P&L to persist: the last observation, else the loaded carry.
    #[must_use]
    pub fn to_persist(&self) -> Option<(NaiveDate, Decimal)> {
        self.last.or(self.carry)
    }

    #[must_use]
    pub fn last(&self) -> Option<Decimal> {
        self.last.map(|(_, p)| p)
    }

    /// The last day P&L, only if it is today's (`now_ms`): a day's figure is not the next day's.
    #[must_use]
    pub fn last_today(&self, now_ms: u64) -> Option<Decimal> {
        self.last.filter(|(day, _)| utc_day(now_ms) <= *day).map(|(_, p)| p)
    }
}

#[cfg(test)]
#[path = "stops_tests.rs"]
mod tests;
