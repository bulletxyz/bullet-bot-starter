//! Past candles: a strategy's [`HistoryNeed`], and the [`Warmup`] the host hands over once, before `start`.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// The most bars the host hands over at start, whatever a [`HistoryNeed`] wants.
pub const MAX_HISTORY_CANDLES: u32 = 999;

/// A candle's length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CandleInterval {
    M1,
    M5,
    M15,
    M30,
    H1,
    H4,
    D1,
}

impl CandleInterval {
    /// Every interval, shortest first.
    pub const ALL: [Self; 7] =
        [Self::M1, Self::M5, Self::M15, Self::M30, Self::H1, Self::H4, Self::D1];

    /// Every interval's [`Self::name`], in the order of [`Self::ALL`].
    pub const NAMES: [&'static str; 7] = [
        Self::M1.name(),
        Self::M5.name(),
        Self::M15.name(),
        Self::M30.name(),
        Self::H1.name(),
        Self::H4.name(),
        Self::D1.name(),
    ];

    /// The usual name (`1m` ... `1d`), as Bullet's and Binance's APIs spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::M1 => "1m",
            Self::M5 => "5m",
            Self::M15 => "15m",
            Self::M30 => "30m",
            Self::H1 => "1h",
            Self::H4 => "4h",
            Self::D1 => "1d",
        }
    }

    /// The interval named `name` (`1m` ... `1d`).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.name() == name)
    }

    /// Length in milliseconds.
    #[must_use]
    pub const fn ms(self) -> u64 {
        const MIN: u64 = 60_000;
        match self {
            Self::M1 => MIN,
            Self::M5 => 5 * MIN,
            Self::M15 => 15 * MIN,
            Self::M30 => 30 * MIN,
            Self::H1 => 60 * MIN,
            Self::H4 => 240 * MIN,
            Self::D1 => 1_440 * MIN,
        }
    }

    /// When the bar holding `now_ms` opened: the live bar a warm-up's last one meets.
    #[must_use]
    pub fn open_of(self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(now_ms.checked_rem(self.ms()).unwrap_or(0))
    }
}

/// One closed candle of a market's past prices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candle {
    /// When it opened (unix ms, a multiple of the interval).
    pub open_ms: u64,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    /// Traded in it, in the base asset (zero when the source does not say).
    pub volume: Decimal,
}

/// The past candles a strategy warms up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryNeed {
    /// The candles' length: the one the strategy builds live.
    pub interval: CandleInterval,
    /// The most bars it can use; the host gives at most [`MAX_HISTORY_CANDLES`].
    pub want: u32,
    /// The fewest it can warm up on: fewer and it gets [`Warmup::Unavailable`].
    pub need: u32,
}

/// What the host loaded for a [`HistoryNeed`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Warmup {
    /// Closed bars, oldest first, with no gap: the last one closed when the bar open at `now_ms`
    /// (which the live data builds) opened. Rebased onto the market's price.
    Loaded {
        candles: Vec<Candle>,
        /// The venue they came from, as a person reads it.
        source: String,
        /// The bot's clock when they were checked.
        now_ms: u64,
    },
    /// None loaded, in plain words why: the strategy warms up from live data.
    Unavailable(String),
}
