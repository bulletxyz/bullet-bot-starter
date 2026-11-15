//! Venue reads and the data a tick refreshes: fees, settled funding (the signal), open interest.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "ms timestamps and the validated 1..=480 h window stay far inside i64; counts are small"
)]

use std::collections::BTreeSet;

use bb_strategy_sdk::{
    Answer, CallError, Decimal, FundingSample, OrderBook, Position, ReadQuery, VenueCall,
};

use super::Core;
use crate::limits::{
    BOOK_DEPTH, FEE_REFRESH_SECS, FEE_RETRY_SECS, FUNDING_FETCHES_PER_TICK, MAX_FUNDING_RATE_8H,
    OI_FETCHES_PER_TICK, OI_MAX_AGE_SECS, OI_REFRESH_SECS, SIGNAL_MAX_AGE_SECS,
    SIGNAL_REFRESH_SECS, SIGNAL_RETRY_SECS,
};
use crate::signal::trailing_median;

const HOUR_MS: i64 = 3_600_000;

fn wrong(what: &str) -> CallError {
    CallError {
        message: format!("unexpected answer to {what}"),
        never_sent: false,
        retryable: false,
    }
}

impl Core {
    async fn read(&self, query: ReadQuery) -> Result<Answer, CallError> {
        self.io.call_on(super::UNIVERSE, VenueCall::Read(query)).await
    }

    /// The venue's positions; `Err` when unreadable (never read as flat).
    pub(crate) async fn positions(&self) -> Result<Vec<Position>, CallError> {
        match self.read(ReadQuery::Positions).await? {
            Answer::Positions(p) => Ok(p),
            _ => Err(wrong("positions")),
        }
    }

    pub(crate) async fn equity(&self) -> Result<Option<Decimal>, CallError> {
        match self.read(ReadQuery::AccountEquity).await? {
            Answer::AccountEquity(e) => Ok(e.map(|e| e.equity)),
            _ => Err(wrong("equity")),
        }
    }

    pub(crate) async fn margin_ratio(&self) -> Result<Option<Decimal>, CallError> {
        match self.read(ReadQuery::MarginRatio).await? {
            Answer::MarginRatio(r) => Ok(r),
            _ => Err(wrong("margin ratio")),
        }
    }

    pub(crate) async fn fresh_book(&self, symbol: &str) -> Option<OrderBook> {
        let query = ReadQuery::Book { symbol: symbol.to_owned(), depth: BOOK_DEPTH };
        match self.read(query).await {
            Ok(Answer::Book(b)) => Some(b),
            _ => None,
        }
    }

    /// Whether `symbol`'s mark and book are both recent (within `stale_secs`).
    pub(crate) fn fresh(&self, symbol: &str, now_ms: u64) -> bool {
        let limit = self.config.stale_secs.saturating_mul(1_000);
        let recent = |at: Option<u64>| {
            at.is_some_and(|t| now_ms.checked_sub(t).is_some_and(|age| age <= limit))
        };
        self.io.with(|s| {
            s.seen.get(symbol).is_some_and(|m| recent(m.mark_at_ms) && recent(m.book_at_ms))
        })
    }

    /// Fees: re-read the venue's metadata hourly (a minute when one is unknown), keep the facts.
    pub(crate) async fn refresh_fees(&mut self) {
        let now = self.now_ms();
        let unknown = self.syms.values().any(|s| s.fee_bps.is_none());
        let every = if unknown { FEE_RETRY_SECS } else { FEE_REFRESH_SECS } * 1_000;
        if self.fee_refresh_last_ms.is_some_and(|t| now.saturating_sub(t) < every) {
            return;
        }
        self.fee_refresh_last_ms = Some(now);
        // A failed refresh keeps the last fees, as the built-in does.
        let _ = self.read(ReadQuery::RefreshMetadata).await;
        let facts = self.io.with(|s| s.facts.clone());
        for (symbol, st) in &mut self.syms {
            st.fee_bps = facts.get(symbol).and_then(|f| f.taker_fee_bps);
        }
    }

    /// The signal: the median of settled funding, held symbols first, a few reads a tick.
    pub(crate) async fn refresh_signals(&mut self, held: &BTreeSet<String>) {
        let now = self.now_ms();
        let now_ms = i64::try_from(now).unwrap_or(i64::MAX);
        let window = self.config.signal_window_hours;
        let spare = i64::try_from(window + 2).unwrap_or(0);
        let start_ms = now_ms - spare * HOUR_MS;
        for st in self.syms.values_mut() {
            let old = st
                .signal_at_ms
                .is_none_or(|t| now.saturating_sub(t) >= SIGNAL_MAX_AGE_SECS * 1_000);
            if st.signal.is_some() && old {
                st.signal = None;
            }
        }
        let mut due: Vec<(bool, Option<u64>, String)> = self
            .syms
            .iter()
            .filter(|(_, s)| s.signal_next_fetch_ms.is_none_or(|t| now >= t))
            .map(|(k, s)| (!held.contains(k), s.signal_next_fetch_ms, k.clone()))
            .collect();
        due.sort();
        for (_, _, symbol) in due.into_iter().take(FUNDING_FETCHES_PER_TICK) {
            let result = self.funding_history(&symbol, start_ms).await;
            let now = self.now_ms();
            let Some(st) = self.syms.get_mut(&symbol) else { continue };
            match result {
                Ok(samples) => {
                    // A settlement past the bad-data cap is dropped alone.
                    let kept: Vec<FundingSample> = samples
                        .into_iter()
                        .filter(|s| s.rate_8h.abs() <= MAX_FUNDING_RATE_8H)
                        .collect();
                    st.signal =
                        trailing_median(&kept, i64::try_from(now).unwrap_or(i64::MAX), window);
                    st.signal_at_ms = st.signal.map(|_| now);
                    st.signal_next_fetch_ms = Some(now + SIGNAL_REFRESH_SECS * 1_000);
                }
                Err(_) => st.signal_next_fetch_ms = Some(now + SIGNAL_RETRY_SECS * 1_000),
            }
        }
    }

    async fn funding_history(
        &self,
        symbol: &str,
        start_ms: i64,
    ) -> Result<Vec<FundingSample>, CallError> {
        let query = ReadQuery::FundingHistory { symbol: symbol.to_owned(), start_ms };
        match self.read(query).await? {
            Answer::FundingHistory(s) => Ok(s),
            _ => Err(wrong("funding history")),
        }
    }

    /// Open interest, held symbols first, a few reads a tick; one past its age is dropped.
    pub(crate) async fn refresh_oi(&mut self, held: &BTreeSet<String>) {
        let now = self.now_ms();
        let age = |at: Option<u64>| at.map(|t| now.saturating_sub(t) / 1_000);
        for st in self.syms.values_mut() {
            if st.oi_base.is_some() && age(st.oi_at_ms).is_none_or(|a| a >= OI_MAX_AGE_SECS) {
                st.oi_base = None;
            }
        }
        let mut due: Vec<(bool, Option<u64>, String)> = self
            .syms
            .iter()
            .filter(|(_, s)| age(s.oi_at_ms).is_none_or(|a| a >= OI_REFRESH_SECS))
            .map(|(k, s)| (!held.contains(k), age(s.oi_at_ms).map(|a| u64::MAX - a), k.clone()))
            .collect();
        due.sort();
        for (_, _, symbol) in due.into_iter().take(OI_FETCHES_PER_TICK) {
            let reply = self.read(ReadQuery::OpenInterest { symbol: symbol.clone() }).await;
            let now = self.now_ms();
            let Some(st) = self.syms.get_mut(&symbol) else { continue };
            st.oi_at_ms = Some(now);
            if let Ok(Answer::OpenInterest(oi)) = reply {
                st.oi_base = oi.filter(|v| *v > Decimal::ZERO);
            }
        }
    }
}
