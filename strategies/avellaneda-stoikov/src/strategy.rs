//! Avellaneda-Stoikov market maker as a [`StrategyAdapter`]: data in, intents out.
//!
//! Two modes, selected by config:
//!   - **Single-venue (default)**: `s` in the A-S formula comes from the trading venue's book mid.
//!     Faithful to the paper.
//!   - **Fair-value MM**: when `reference_exchange` is set, `s` comes from the reference feed
//!     ([`Event::Reference`], e.g. Binance). The local book is still used for would-cross checks
//!     but no longer drives `last_mid` or the volatility estimator. The textbook A-S inventory skew
//!     (`r = s − q·γ·σ²·τ`) is unchanged — only the source of `s`.
//!
//! Events:
//!   - `Book` — drives `last_mid`/volatility only when no reference is configured. Always triggers
//!     a refresh attempt so we can react to inventory-cap toggles even without a reference.
//!   - `Reference` — when a reference is configured, the canonical source of `s` and the volatility
//!     estimator's input.
//!   - `Fill` — the host has already recorded it in the inventory (`View::position`, the canonical
//!     source). Drops the filled slot and forces a re-quote so the next tick re-builds the ladder
//!     around the new reservation price.
//!   - `Order` — learns the exchange order id; a terminal status drops the slot.
//!   - `Outcome` / `Failed` / `Unknown` — the answers to the round's cancels, amends and places
//!     (`strategy/sync.rs`).
//!   - `Tick` — the periodic reconcile with the venue's open orders, and the fallback refresh when
//!     `order_refresh_secs` elapses, so calm markets are still re-quoted.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal USD notionals, prices, sizes and bps sit far below 7.9e28; constant divisors; \
              slot counts where the subtrahend is a filtered subset"
)]

use std::collections::HashMap;

use bb_strategy_sdk::{Event, Intent, Side, StrategyAdapter, StrategyError, View};
use bb_strategy_support::ClientIdIssuer;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::config::AvellanedaStoikovConfig;
use crate::report::{AvellanedaStoikovReport, QuoteReport};
use crate::strategy::slots::{InFlight, QuoteSlot};
use crate::volatility::Volatility;

mod quoting;
pub(crate) mod slots;
mod sync;

pub struct AvellanedaStoikov {
    pub(crate) config: AvellanedaStoikovConfig,
    pub(crate) slots: Vec<QuoteSlot>,
    /// The cancels, amends and places of the last refresh not yet answered.
    pub(crate) in_flight: Vec<InFlight>,
    /// Quotes held for a place whose outcome is unknown: the client id and the bot clock (ms) of
    /// the event. A venue read frees one only if it began at least `UNKNOWN_SETTLE_MS` later.
    pub(crate) unknown: HashMap<String, u64>,
    /// When the round in flight began (bot clock, ms) and whether any call of it failed as a
    /// whole.
    pub(crate) round: Option<(u64, bool)>,
    pub(crate) client_ids: ClientIdIssuer,
    pub(crate) volatility: Volatility,
    pub(crate) last_mid: Option<Decimal>,
    pub(crate) last_quote_at_ms: Option<u64>,
    /// Set when `reference_exchange` is configured. When `Some`, `last_mid` and the volatility
    /// estimator are driven exclusively by the reference feed.
    pub(crate) reference_last_seen_ms: Option<u64>,
    pub(crate) last_reconcile_at_ms: Option<u64>,
}

impl AvellanedaStoikov {
    pub fn new(config: AvellanedaStoikovConfig) -> Self {
        let volatility = Volatility::new(config.vol_window_secs);
        Self {
            config,
            slots: Vec::new(),
            in_flight: Vec::new(),
            unknown: HashMap::new(),
            round: None,
            client_ids: ClientIdIssuer::default(),
            volatility,
            last_mid: None,
            last_quote_at_ms: None,
            reference_last_seen_ms: None,
            last_reconcile_at_ms: None,
        }
    }

    pub(crate) fn uses_reference(&self) -> bool {
        self.config.reference_exchange.is_some()
    }

    pub(crate) fn reference_is_stale(&self, now_ms: u64) -> bool {
        match self.reference_last_seen_ms {
            None => true,
            Some(t) => {
                now_ms.saturating_sub(t)
                    > self.config.reference_stale_secs.max(1).saturating_mul(1000)
            }
        }
    }

    fn check_fee_floor(&self) -> Result<(), StrategyError> {
        let Some(fees) = &self.config.fees else {
            tracing::warn!(
                "No [strategy.avellaneda-stoikov.fees] configured — skipping fee-floor check."
            );
            return Ok(());
        };
        let round_trip_bps = Decimal::from(2) * fees.maker_bps;
        let required_bps = round_trip_bps * fees.min_spread_fee_multiplier;
        let min_full_spread_bps = Decimal::from(2) * self.config.min_half_spread_bps;

        tracing::info!(
            min_full_spread_bps = %min_full_spread_bps,
            required_bps = %required_bps,
            maker_bps = %fees.maker_bps,
            "A-S fee-floor check"
        );

        if min_full_spread_bps < required_bps {
            return Err(StrategyError(format!(
                "min_half_spread_bps × 2 = {min_full_spread_bps} bps is below required \
                 {required_bps} bps ({} × round-trip maker fee). Raise min_half_spread_bps.",
                fees.min_spread_fee_multiplier
            )));
        }
        Ok(())
    }

    fn on_book(&mut self, view: &View<'_>) -> Vec<Intent> {
        // In single-venue mode the local book is the price source. In fair-value mode the
        // reference feed owns `last_mid` / volatility.
        if !self.uses_reference()
            && let Some(m) = view.book.and_then(bb_strategy_sdk::OrderBook::midpoint)
        {
            self.last_mid = Some(m);
            if let Some(f) = m.to_f64() {
                self.volatility.push(f, view.now_ms);
            }
        }
        // Refresh on every book update; the function self-throttles via `amend_threshold_bps`, so
        // calls where nothing has moved are free.
        match self.last_mid {
            Some(m) => self.refresh_quotes(view, m),
            None => Vec::new(),
        }
    }

    fn on_reference(
        &mut self,
        price: &bb_strategy_sdk::ReferencePrice,
        view: &View<'_>,
    ) -> Vec<Intent> {
        let Some(ref_symbol) = self.config.reference_symbol.as_deref() else {
            return Vec::new();
        };
        if !price.symbol.eq_ignore_ascii_case(ref_symbol) {
            return Vec::new();
        }
        self.reference_last_seen_ms = Some(price.received_ms);
        let prev = self.last_mid;
        if prev != Some(price.mid) {
            tracing::debug!(
                prev = ?prev.map(|p| p.to_string()),
                new = %price.mid,
                "ref mid changed"
            );
        }
        self.last_mid = Some(price.mid);
        if let Some(f) = price.mid.to_f64() {
            self.volatility.push(f, view.now_ms);
        }
        self.refresh_quotes(view, price.mid)
    }

    fn on_tick(&mut self, view: &View<'_>) -> Vec<Intent> {
        let Some(mid) = self.last_mid else {
            return Vec::new();
        };
        let mut intents = Vec::new();
        if self.in_flight.is_empty() {
            if let Some(open) = view.open_orders
                && self.reconcile_due(view.now_ms, view.reconnected)
            {
                intents = self.reconcile_with_venue(open, view);
            }
            if self.should_force_refresh(view.now_ms) {
                intents.extend(self.refresh_quotes(view, mid));
            }
        }

        let best_bid = self.slots.iter().filter(|s| s.side == Side::Buy).map(|s| s.price).max();
        let top_ask = self.slots.iter().filter(|s| s.side == Side::Sell).map(|s| s.price).min();
        tracing::info!(
            net_pos = %view.position,
            pnl = %view.realized_pnl,
            slots = self.slots.len(),
            mid = ?self.last_mid.map(|p| p.to_string()),
            best_bid = ?best_bid.map(|p| p.to_string()),
            best_ask = ?top_ask.map(|p| p.to_string()),
            vol_samples = self.volatility.sample_count(),
            "A-S tick"
        );
        intents
    }
}

impl StrategyAdapter for AvellanedaStoikov {
    type Report = AvellanedaStoikovReport;

    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        self.config
            .validate()
            .map_err(|e| StrategyError(format!("avellaneda-stoikov config invalid: {e}")))?;
        self.check_fee_floor()?;
        // The host has cancelled the stale orders and seeded the inventory from the venue.
        self.client_ids = ClientIdIssuer::own(view.now_ms);
        // Seed mid/vol only in single-venue mode (in fair-value mode the reference feed owns
        // those).
        if !self.uses_reference() {
            let mid =
                view.book.and_then(bb_strategy_sdk::OrderBook::midpoint).ok_or_else(|| {
                    StrategyError("No orderbook data available to compute mid price".into())
                })?;
            self.last_mid = Some(mid);
            if let Some(m) = mid.to_f64() {
                self.volatility.push(m, view.now_ms);
            }
        }
        tracing::info!(
            mode = if self.uses_reference() { "fair-value" } else { "single-venue" },
            reference = ?self.config.reference_exchange,
            net_pos = %view.position,
            gamma = %self.config.gamma,
            kappa = %self.config.kappa,
            tau = self.config.order_horizon_secs,
            "A-S started — awaiting price samples before first quote"
        );
        Ok(Vec::new())
    }

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent> {
        match event {
            Event::Book => self.on_book(view),
            Event::Reference(price) => self.on_reference(price, view),
            Event::Fill(trade) => {
                self.on_fill(trade);
                Vec::new()
            }
            Event::Order(order) => {
                self.on_order(order);
                Vec::new()
            }
            Event::Tick => self.on_tick(view),
            Event::Outcome { intent, result } => {
                self.on_outcome(intent, result, view.now_ms);
                Vec::new()
            }
            Event::Failed { intent, .. } => {
                self.on_not_sent(intent);
                Vec::new()
            }
            Event::Unknown { intent, .. } => {
                self.on_unknown(intent, view.now_ms);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn stop(&mut self, view: &View<'_>) -> Vec<Intent> {
        // The wind-down reason is ignored: market-making never wants to take taker fees at
        // shutdown, so cancel-only is correct for every reason including a feed failure.
        tracing::info!("Shutting down A-S — cancelling all orders");
        tracing::info!(
            net_pos = %view.position,
            pnl = %view.realized_pnl,
            "A-S final stats"
        );
        vec![Intent::CancelAll]
    }

    fn report(&self) -> AvellanedaStoikovReport {
        AvellanedaStoikovReport {
            slots: self.slots.iter().map(QuoteReport::from).collect(),
            sigma: self.volatility.sigma(),
            vol_samples: self.volatility.sample_count(),
        }
    }

    fn orders_cancelled(&mut self) {
        self.slots.clear();
    }

    /// The venue's open orders are read at a tick only when the reconcile is due (or the stream
    /// reconnected): the rest of the time the strategy does not look at them.
    fn wants_venue(&self, now_ms: u64, reconnected: bool) -> bool {
        self.reconcile_due(now_ms, reconnected)
    }
}

#[cfg(test)]
#[path = "strategy_tests.rs"]
mod tests;
