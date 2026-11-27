//! An example indicator written only against `bb-strategy-sdk`: a simple moving average of the
//! market's closes over `period` bars of `interval`. It warms up at start on the past candles the
//! host loads (`history` and `warm_up`, the 0.5.0 world), so it has a value from its first event,
//! after a restart too; without them it warms up live. Live, each bar closes at the last mid it
//! saw when the clock moves into the next one. It places no orders: it reports the average and the
//! price against it, a starting point for a strategy that trades on one. Built for
//! `wasm32-wasip2` it is a `bb:strategy` component (`just wasm`).

use std::collections::VecDeque;

use bb_strategy_sdk::{
    BULLET, CandleInterval, Decimal, Describe, Descriptor, Event, ExchangeNeed, FromSettings,
    HistoryNeed, Intent, MAX_FIELD_TEXT, MarketView, OrderBook, Range, ReportField, SettingKind,
    SettingSpec, StrategyAdapter, StrategyError, StrategyReport, View, Warmup,
};
use serde::Deserialize;

/// The most bars it averages.
pub const MAX_PERIOD: u32 = 500;

/// The settings, as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SmaSettings {
    /// Bars averaged.
    pub period: u32,
    /// Each bar's length: `1m`, `5m`, `15m`, `30m`, `1h`, `4h` or `1d`.
    #[serde(default = "fifteen_minutes")]
    pub interval: String,
}

fn fifteen_minutes() -> String {
    "15m".into()
}

/// The moving average.
#[derive(Debug, Clone)]
pub struct Sma {
    period: u32,
    interval: CandleInterval,
    name: String,
    /// Closed bars' closes, oldest first, at most `period`.
    closes: VecDeque<Decimal>,
    /// The bar being built live: when it opened, and the last mid in it.
    live: Option<(u64, Decimal)>,
    /// How it warmed up, in plain words.
    warmed: String,
}

impl Sma {
    /// # Errors
    /// Settings it cannot run with.
    pub fn new(settings: &SmaSettings) -> Result<Self, StrategyError> {
        if settings.period < 2 || settings.period > MAX_PERIOD {
            return Err(StrategyError(format!(
                "sma: period must be 2 to {MAX_PERIOD}, got {}",
                settings.period
            )));
        }
        let Some(interval) = CandleInterval::parse(&settings.interval) else {
            return Err(StrategyError(format!(
                "sma: interval must be one of {}, got {:?}",
                CandleInterval::NAMES.join(", "),
                settings.interval
            )));
        };
        Ok(Self {
            period: settings.period,
            interval,
            name: settings.interval.clone(),
            closes: VecDeque::new(),
            live: None,
            warmed: "not yet".into(),
        })
    }

    /// The average over the last `period` closes, once it has them.
    #[must_use]
    pub fn average(&self) -> Option<Decimal> {
        if self.closes.len() < self.period as usize {
            return None;
        }
        let sum = self.closes.iter().try_fold(Decimal::ZERO, |s, c| s.checked_add(*c))?;
        sum.checked_div(Decimal::from(self.period))
    }

    /// The closed bars it holds.
    #[must_use]
    pub fn bars(&self) -> usize {
        self.closes.len()
    }

    fn close_bar(&mut self, close: Decimal) {
        self.closes.push_back(close);
        while self.closes.len() > self.period as usize {
            self.closes.pop_front();
        }
    }

    /// The live bar moves to the mid of its market (Bullet's one); the one before closes once the
    /// clock leaves it.
    fn see(&mut self, view: &View<'_>) {
        let Some(mid) = mid(view.sole_market(BULLET).and_then(MarketView::book)) else { return };
        let open = self.interval.open_of(view.now_ms);
        match self.live {
            Some((at, close)) if open > at => self.close_bar(close),
            // The clock never goes back; a view from before the live bar changes nothing.
            Some((at, _)) if open < at => return,
            _ => {}
        }
        self.live = Some((open, mid));
    }

    fn price(&self) -> Option<Decimal> {
        self.live.map(|(_, mid)| mid)
    }
}

fn mid(book: Option<&OrderBook>) -> Option<Decimal> {
    let book = book?;
    let (bid, ask) = (book.bids.last_key_value()?.0, book.asks.first_key_value()?.0);
    bid.checked_add(*ask)?.checked_div(Decimal::TWO)
}

fn cut(text: &str) -> String {
    text.chars().take(MAX_FIELD_TEXT).collect()
}

impl Describe for Sma {
    const NAME: &'static str = "sma";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn describe() -> Descriptor {
        let d = |n: i64| Decimal::from(n);
        let interval = SettingSpec {
            kind: SettingKind::Choice(CandleInterval::NAMES.iter().map(|n| (*n).into()).collect()),
            ..SettingSpec::int("interval", 2, Range::new(d(0), d(6)), "Each bar's length.")
        };
        Descriptor {
            summary: "Shows a moving average of the price, warmed up on past candles at start. \
                      Places no orders."
                .into(),
            settings: vec![
                SettingSpec::int(
                    "period",
                    20,
                    Range::new(d(2), d(i64::from(MAX_PERIOD))),
                    "Bars averaged.",
                )
                .required(),
                interval,
            ],
            settings_schema: schemars::schema_for!(SmaSettings),
            exchanges: vec![ExchangeNeed::new(BULLET)],
        }
    }
}

impl FromSettings for Sma {
    fn from_settings(settings: &str) -> Result<Self, StrategyError> {
        let settings: SmaSettings =
            serde_json::from_str(settings).map_err(|e| StrategyError(format!("sma: {e}")))?;
        Self::new(&settings)
    }
}

/// What it shows: the average, the price against it, and how it warmed up.
#[derive(Debug, Clone)]
pub struct SmaReport {
    pub period: u32,
    pub interval: String,
    pub bars: usize,
    pub average: Option<Decimal>,
    pub price: Option<Decimal>,
    pub warmed: String,
}

impl StrategyReport for SmaReport {
    fn headline(&self) -> String {
        let name = format!("SMA({}, {})", self.period, self.interval);
        match (self.average, self.price) {
            (Some(avg), Some(price)) => {
                let side = if price >= avg { "above" } else { "below" };
                format!("{name} {}: the price is {side} it", avg.round_dp(8).normalize())
            }
            (Some(avg), None) => format!("{name} {}", avg.round_dp(8).normalize()),
            (None, _) => format!("{name} warming up: {} of {} bars", self.bars, self.period),
        }
    }

    fn fields(&self) -> Vec<ReportField> {
        let mut fields = vec![
            ReportField::number("bars", Decimal::from(self.bars)),
            ReportField::number("period", Decimal::from(self.period)),
            ReportField::text("interval", self.interval.clone()),
            ReportField::text("warmed_up", cut(&self.warmed)),
        ];
        fields.extend(self.average.map(|a| ReportField::number("sma", a)));
        fields.extend(self.price.map(|p| ReportField::number("price", p)));
        if let (Some(avg), Some(price)) = (self.average, self.price) {
            fields.push(ReportField::flag("above", price >= avg));
        }
        fields
    }
}

impl StrategyAdapter for Sma {
    type Report = SmaReport;

    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        self.see(view);
        Ok(Vec::new())
    }

    fn on_event(&mut self, _event: &Event, view: &View<'_>) -> Vec<Intent> {
        self.see(view);
        Vec::new()
    }

    fn stop(&mut self, _view: &View<'_>) -> Vec<Intent> {
        Vec::new()
    }

    fn report(&self) -> SmaReport {
        SmaReport {
            period: self.period,
            interval: self.name.clone(),
            bars: self.bars(),
            average: self.average(),
            price: self.price(),
            warmed: self.warmed.clone(),
        }
    }

    /// Exactly `period` bars: fewer and it warms up live.
    fn history(&self) -> Option<HistoryNeed> {
        Some(HistoryNeed { interval: self.interval, want: self.period, need: self.period })
    }

    fn warm_up(&mut self, warmup: Warmup) {
        self.warmed = match warmup {
            Warmup::Loaded { candles, source, .. } => {
                for candle in &candles {
                    self.close_bar(candle.close);
                }
                format!("{} past bars from {source}", candles.len())
            }
            Warmup::Unavailable(why) => format!("live: no past bars ({why})"),
        };
    }
}

bb_strategy_sdk::export_strategy!(Sma);

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
