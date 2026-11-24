//! The pricing: one bid and one ask at a fair value ± the side's spread (wider while volatile, if
//! the switch is on), shifted by inventory skew, kept off the touch. The fair value is the Binance
//! mid plus the basis, or the Bullet mid ([`fair_value`]). What it quotes still passes the
//! market maker's own cap and post-only check (`adapter/quoting.rs`).

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal prices, sizes and bps sit far below 7.9e28; max_position checked > 0 before \
              dividing"
)]

use std::collections::VecDeque;

use bb_strategy_sdk::{OrderBook, ReferencePrice, Side};
use rust_decimal::Decimal;

use crate::params::{FairValue, MmParams};
use crate::settings::{VOL_THRESHOLD_BPS, VOL_WIDEN_BPS, VOL_WIDEN_ENABLED, VOL_WINDOW_SECS};

pub mod fair_value;

use fair_value::FairValueTracker;
pub use fair_value::Fallback;

const BPS_SCALE: i64 = 10_000;

/// What the pricing sees of the market and of the bot.
#[derive(Debug, Clone, Copy)]
pub struct MarketView<'a> {
    /// The traded venue's book, when a usable one is held.
    pub book: Option<&'a OrderBook>,
    pub mid: Option<Decimal>,
    /// The bot clock, Unix ms.
    pub now_ms: u64,
    pub net_position: Decimal,
    pub max_position: Decimal,
    pub params: &'a MmParams,
    /// The venue's price increment, when known.
    pub tick: Option<Decimal>,
}

impl MarketView<'_> {
    /// Keep `price` from crossing the book: a sell at least one tick above the best bid, a buy at
    /// least one tick below the best ask. Skew can push a quote past the touch on a tight book;
    /// clamping keeps the position-reducing side quoting instead of being dropped. Without a known
    /// tick, the floor is the opposite touch itself (joins it; never crosses).
    #[must_use]
    pub fn keep_off_touch(&self, side: Side, price: Decimal) -> Decimal {
        let Some(book) = self.book else { return price };
        let tick = self.tick.filter(|t| *t > Decimal::ZERO);
        match side {
            Side::Sell => match (book.best_bid(), tick, book.best_ask()) {
                (Some(bid), Some(t), _) => price.max(bid.price + t),
                (_, None, Some(ask)) if book.would_cross(side, price) => ask.price,
                _ => price,
            },
            Side::Buy => match (book.best_ask(), tick, book.best_bid()) {
                (Some(ask), Some(t), _) => price.min(ask.price - t),
                (_, None, Some(bid)) if book.would_cross(side, price) => bid.price,
                _ => price,
            },
        }
    }
}

/// One optional quote per side, and the fair value they were priced from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Quotes {
    pub bid: Option<Quote>,
    pub ask: Option<Quote>,
    pub fair: Option<FairReport>,
}

/// One wanted order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quote {
    pub price: Decimal,
    pub size: Decimal,
}

/// The fair value quotes are centred on, for the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FairReport {
    pub price: Decimal,
    pub source: FairValue,
    /// Set when the configured source is not used, and why.
    pub fallback: Option<Fallback>,
    /// Basis to the reference venue in bps, once there is one.
    pub basis_bps: Option<Decimal>,
}

/// The market maker's pricing.
#[derive(Debug)]
pub struct Pricing {
    /// Binance mid and basis EMA behind the fair value.
    fair: FairValueTracker,
    /// Binance symbol for the fair value (upper case).
    binance_symbol: String,
    /// Recent Bullet mids (bot clock ms), oldest first, kept while the volatility switch is on.
    mids: VecDeque<(u64, Decimal)>,
}

impl Pricing {
    #[must_use]
    pub fn new(binance_symbol: String) -> Self {
        Self { fair: FairValueTracker::default(), binance_symbol, mids: VecDeque::new() }
    }

    fn vol_window_ms(params: &MmParams) -> u64 {
        u64::try_from(params.own(VOL_WINDOW_SECS)).unwrap_or(0).saturating_mul(1000)
    }

    /// Keep the Bullet mids within the window while the switch is on.
    fn track_mid(&mut self, mid: Decimal, view: &MarketView<'_>) {
        if !view.params.flag(VOL_WIDEN_ENABLED) {
            self.mids.clear();
            return;
        }
        let window = Self::vol_window_ms(view.params);
        while self.mids.front().is_some_and(|(t, _)| view.now_ms.saturating_sub(*t) > window) {
            self.mids.pop_front();
        }
        self.mids.push_back((view.now_ms, mid));
    }

    /// Bps added to both spreads: `vol_widen_bps` while the switch is on and the mid's high-low
    /// range within the window reaches `vol_threshold_bps`.
    fn vol_widen_bps(&self, view: &MarketView<'_>) -> Decimal {
        if !view.params.flag(VOL_WIDEN_ENABLED) {
            return Decimal::ZERO;
        }
        let window = Self::vol_window_ms(view.params);
        let recent = || {
            self.mids
                .iter()
                .filter(move |(t, _)| view.now_ms.saturating_sub(*t) <= window)
                .map(|(_, m)| *m)
        };
        let (Some(lo), Some(hi)) = (recent().min(), recent().max()) else {
            return Decimal::ZERO;
        };
        if lo <= Decimal::ZERO {
            return Decimal::ZERO;
        }
        let range_bps = (hi - lo) / lo * Decimal::from(BPS_SCALE);
        if range_bps >= view.params.own(VOL_THRESHOLD_BPS) {
            view.params.own(VOL_WIDEN_BPS)
        } else {
            Decimal::ZERO
        }
    }

    /// Inventory skew in bps applied to both quotes: `-skew_bps × net / max_position`, the ratio
    /// clamped to ±1. Long lowers both quotes.
    fn skew_shift_bps(view: &MarketView<'_>) -> Decimal {
        if view.params.skew_bps.is_zero() || view.max_position <= Decimal::ZERO {
            return Decimal::ZERO;
        }
        let ratio =
            (view.net_position / view.max_position).clamp(Decimal::NEGATIVE_ONE, Decimal::ONE);
        -view.params.skew_bps * ratio
    }

    /// The quote price for `side`: `fair` ± the side's spread (plus `widen_bps`), shifted by the
    /// skew, then kept off the touch.
    fn desired_price(
        view: &MarketView<'_>,
        side: Side,
        fair: Decimal,
        widen_bps: Decimal,
    ) -> Decimal {
        let spread_bps = match side {
            Side::Buy => -(view.params.bid_spread_bps + widen_bps),
            Side::Sell => view.params.ask_spread_bps + widen_bps,
        };
        let price = fair
            * (Decimal::ONE + (spread_bps + Self::skew_shift_bps(view)) / Decimal::from(BPS_SCALE));
        view.keep_off_touch(side, price)
    }

    /// A fresh, usable book for the traded symbol (`view.mid` is set).
    pub fn on_book(&mut self, view: &MarketView<'_>) {
        let Some(mid) = view.mid else { return };
        // A sustained shift restarts the warm-up; the report shows the fallback meanwhile.
        let _relearned = self.fair.on_bullet_mid(mid, view.now_ms, view.params);
        self.track_mid(mid, view);
    }

    /// The reference venue's best bid and ask. Only recorded: quotes move on the next Bullet book
    /// or tick. Ignored while pricing off Bullet: a switch to binance starts a fresh warm-up.
    pub fn on_reference(&mut self, price: &ReferencePrice, view: &MarketView<'_>) {
        if view.params.fair_value == FairValue::Binance
            && price.symbol.eq_ignore_ascii_case(&self.binance_symbol)
        {
            // Stamped with the feed's receive time, so a backlog reads as stale.
            self.fair.on_reference(price.bid, price.ask, price.received_ms.min(view.now_ms));
        }
    }

    /// The live params changed from `before` to `after`.
    pub fn on_params(&mut self, before: &MmParams, after: &MmParams) {
        if before.fair_value != FairValue::Binance && after.fair_value == FairValue::Binance {
            // Never reuse a basis warmed before the switch away from binance.
            self.fair = FairValueTracker::default();
        }
    }

    /// The quotes wanted now.
    #[must_use]
    pub fn quote(&self, view: &MarketView<'_>) -> Quotes {
        let Some(fair) = self.fair(view) else { return Quotes::default() };
        let widen = self.vol_widen_bps(view);
        let quote = |side| Quote {
            price: Self::desired_price(view, side, fair.price, widen),
            size: view.params.order_size,
        };
        Quotes { bid: Some(quote(Side::Buy)), ask: Some(quote(Side::Sell)), fair: Some(fair) }
    }

    /// The fair value for the report, when there is a Bullet mid.
    #[must_use]
    pub fn fair(&self, view: &MarketView<'_>) -> Option<FairReport> {
        let mid = view.mid?;
        let fair = self.fair.fair(mid, view.now_ms, view.params);
        Some(FairReport {
            price: fair.price,
            source: fair.source,
            fallback: fair.fallback,
            basis_bps: self.fair.basis_bps(mid),
        })
    }
}

#[cfg(test)]
#[path = "pricing_tests.rs"]
mod tests;
