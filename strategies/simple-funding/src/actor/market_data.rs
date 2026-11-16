use std::collections::{HashMap, HashSet};
use std::time::Duration;

use bb_strategy_support::trailing_median;
use rust_decimal::Decimal;

use super::{HOUR_MS, SimpleFundingActor, usd};
use crate::adapter::io::{Mono, Venue};
use crate::cost::{assess, fillable_within};
use crate::decide::{MarketView, SymbolView};
use crate::limits::{
    FEE_REFRESH_SECS, FEE_RETRY_SECS, FUNDING_FETCHES_PER_TICK, MAX_FUNDING_RATE_8H,
    OI_FETCHES_PER_TICK, OI_MAX_AGE_SECS, OI_REFRESH_SECS, SIGNAL_MAX_AGE_SECS,
    SIGNAL_REFRESH_SECS, SIGNAL_RETRY_SECS,
};

impl SimpleFundingActor {
    // -- data -------------------------------------------------------------------------

    pub(super) async fn refresh_fees(&mut self, cx: &Venue, b: &Venue) {
        let now = cx.now();
        let unknown = self.syms.values().any(|s| s.fee_bps.is_none());
        let every = Duration::from_secs(if unknown { FEE_RETRY_SECS } else { FEE_REFRESH_SECS });
        if self
            .fee_refresh_last
            .is_some_and(|t| now.checked_duration_since(t).is_some_and(|a| a < every))
        {
            return;
        }
        self.fee_refresh_last = Some(now);
        if let Err(e) = b.refresh_metadata().await {
            tracing::warn!(error = %e, "simple-funding: fee refresh failed, keeping the last fees");
        }
        for (symbol, st) in &mut self.syms {
            let fee = b.taker_fee_bps(symbol);
            if fee.is_none() && st.fee_bps.is_some() {
                tracing::warn!(%symbol, "simple-funding: taker fee now unknown, entries blocked");
            }
            st.fee_bps = fee;
        }
    }

    pub(super) async fn refresh_signals(&mut self, cx: &Venue, b: &Venue, held: &HashSet<String>) {
        let now = cx.now();
        let now_ms = i64::try_from(cx.unix_ms()).unwrap_or(i64::MAX);
        let window = self.config.signal_window_hours;
        // Two spare hours so the oldest in-window settlement is never cut off.
        let start_ms = now_ms - i64::try_from(window + 2).unwrap_or(0) * HOUR_MS;
        for (symbol, st) in &mut self.syms {
            let old = st.signal_at.is_none_or(|t| {
                now.checked_duration_since(t).is_some_and(|a| a.as_secs() >= SIGNAL_MAX_AGE_SECS)
            });
            if st.signal.is_some() && old {
                tracing::warn!(%symbol, "simple-funding: funding signal too old, dropped");
                st.signal = None;
            }
        }
        // Held symbols first (their exits read the signal), then the longest waiting.
        let mut due: Vec<(bool, Option<Mono>, String)> = self
            .syms
            .iter()
            .filter(|(_, s)| s.signal_next_fetch.is_none_or(|t| now >= t))
            .map(|(k, s)| (!held.contains(k), s.signal_next_fetch, k.clone()))
            .collect();
        due.sort();
        for (_, _, symbol) in due.into_iter().take(FUNDING_FETCHES_PER_TICK) {
            let result = b.funding_history(&symbol, start_ms).await;
            let Some(st) = self.syms.get_mut(&symbol) else { continue };
            let after = |secs: u64| now.checked_add(Duration::from_secs(secs)).unwrap_or(now);
            match result {
                Ok(samples) => {
                    // A settlement past the bad-data cap is dropped alone; the median then
                    // ignores the odd real spike a mean would chase.
                    let total = samples.len();
                    let kept: Vec<_> = samples
                        .into_iter()
                        .filter(|s| s.rate_8h.abs() <= MAX_FUNDING_RATE_8H)
                        .collect();
                    if kept.len() < total {
                        tracing::warn!(
                            %symbol, dropped = total - kept.len(),
                            "simple-funding: absurd funding rates dropped from the signal"
                        );
                    }
                    st.signal = trailing_median(&kept, now_ms, window);
                    st.signal_at = st.signal.map(|_| now);
                    st.signal_next_fetch = Some(after(SIGNAL_REFRESH_SECS));
                }
                Err(e) => {
                    tracing::warn!(%symbol, error = %e, "simple-funding: funding history failed, keeping the last signal");
                    st.signal_next_fetch = Some(after(SIGNAL_RETRY_SECS));
                }
            }
        }
    }

    pub(super) async fn refresh_oi(&mut self, cx: &Venue, b: &Venue, held: &HashSet<String>) {
        let now = cx.now();
        let age =
            |at: Option<Mono>| at.and_then(|t| now.checked_duration_since(t)).map(|d| d.as_secs());
        for st in self.syms.values_mut() {
            if st.oi_base.is_some() && age(st.oi_at).is_none_or(|a| a >= OI_MAX_AGE_SECS) {
                st.oi_base = None;
            }
        }
        let mut due: Vec<(bool, Option<u64>, String)> = self
            .syms
            .iter()
            .filter(|(_, s)| age(s.oi_at).is_none_or(|a| a >= OI_REFRESH_SECS))
            .map(|(k, s)| (!held.contains(k), age(s.oi_at).map(|a| u64::MAX - a), k.clone()))
            .collect();
        due.sort();
        for (_, _, symbol) in due.into_iter().take(OI_FETCHES_PER_TICK) {
            let result = b.open_interest(&symbol).await;
            let Some(st) = self.syms.get_mut(&symbol) else { continue };
            st.oi_at = Some(now);
            match result {
                Ok(oi) => st.oi_base = oi.filter(|v| *v > Decimal::ZERO),
                Err(e) => {
                    tracing::warn!(%symbol, error = %e, "simple-funding: open interest read failed");
                }
            }
        }
    }

    // -- view -------------------------------------------------------------------------

    pub(super) fn market_view(
        &self,
        now: Mono,
        equity: Option<Decimal>,
        held_base: &HashMap<String, Decimal>,
        marks: &HashMap<String, Decimal>,
        foreign_gross_usd: Decimal,
    ) -> MarketView {
        let symbols = self
            .syms
            .iter()
            .map(|(symbol, st)| {
                let mark = st.mark.unwrap_or(Decimal::ZERO);
                let price = if mark > Decimal::ZERO {
                    mark
                } else {
                    marks.get(symbol).copied().unwrap_or(Decimal::ZERO)
                };
                let cost = st
                    .book
                    .as_deref()
                    .map(|b| assess(b, st.fee_bps, self.config.notional_per_symbol));
                // The walk the cost was judged on must fit inside the IOC cap, else every
                // entry fills part of its size: thin depth then reads as none.
                let fits = st.book.as_deref().is_some_and(|b| {
                    fillable_within(b, self.config.notional_per_symbol, self.config.slippage_bps)
                });
                SymbolView {
                    symbol: symbol.clone(),
                    signal_8h: st.signal,
                    round_trip_cost: cost.and_then(|c| c.round_trip),
                    mark,
                    oi_usd: st.oi_base.and_then(|oi| oi.checked_mul(mark)),
                    depth_usd: cost.and_then(|c| c.depth_usd).filter(|_| fits),
                    fresh: self.fresh(st, now),
                    held_usd: usd(held_base.get(symbol).copied().unwrap_or(Decimal::ZERO), price),
                    entry_cost: st.entry_cost,
                }
            })
            .collect();
        MarketView { symbols, equity, foreign_gross_usd, params: self.params() }
    }
}
