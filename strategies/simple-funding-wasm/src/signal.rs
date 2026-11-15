//! The funding signal: the median of one market's settled funding over a trailing window, one
//! sample per hour. This crate's own implementation of what the built-in computes (a median, so
//! one spike among floor settlements does not move it where a mean would chase it).

use bb_strategy_sdk::FundingSample;
use rust_decimal::Decimal;

const HOUR_MS: i64 = 3_600_000;

/// Median of the settled `rate_8h` of the last `window_hours` hours up to `now_ms`, the first
/// sample of each hour counting. `None` below three quarters of the window's hours covered.
#[must_use]
pub fn trailing_median(
    samples: &[FundingSample],
    now_ms: i64,
    window_hours: u64,
) -> Option<Decimal> {
    let window = i64::try_from(window_hours).ok()?;
    let newest = now_ms.div_euclid(HOUR_MS);
    let mut hours = std::collections::BTreeSet::new();
    let mut rates: Vec<Decimal> = samples
        .iter()
        .filter_map(|s| {
            let hour = s.time_ms.div_euclid(HOUR_MS);
            let inside = hour > newest.saturating_sub(window) && hour <= newest;
            (inside && hours.insert(hour)).then_some(s.rate_8h)
        })
        .collect();
    let n = i64::try_from(rates.len()).ok()?;
    if n == 0 || n.saturating_mul(4) < window.saturating_mul(3) {
        return None;
    }
    rates.sort();
    let mid = rates.len() / 2;
    if rates.len() % 2 == 1 {
        rates.get(mid).copied()
    } else {
        let (lo, hi) = (rates.get(mid.checked_sub(1)?)?, rates.get(mid)?);
        lo.checked_add(*hi)?.checked_div(Decimal::TWO)
    }
}
