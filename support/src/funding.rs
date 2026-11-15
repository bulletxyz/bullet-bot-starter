//! Pure funding math the funding strategies and the bot's executor share: the taker's walk through
//! a book, and the median of settled funding over a trailing window.

use bb_strategy_sdk::FundingSample;
use rust_decimal::Decimal;

const HOUR_MS: i64 = 3_600_000;

/// A taker's average fill past `mid` for `qty`, in bp, walking `levels` (best first;
/// asks for a buy, bids for a sell). `None` = the book can't fill `qty`, or the walk
/// overflows `Decimal` (absurd venue levels). A crossed or locked book's negative walk
/// counts as 0.
pub fn walk_slip_bps(
    levels: &[(Decimal, Decimal)],
    buy: bool,
    mid: Decimal,
    qty: Decimal,
) -> Option<Decimal> {
    let (mut cost, mut filled) = (Decimal::ZERO, Decimal::ZERO);
    for &(px, q) in levels {
        let take = q.min(qty.checked_sub(filled)?);
        if take <= Decimal::ZERO {
            break;
        }
        (cost, filled) = (cost.checked_add(px.checked_mul(take)?)?, filled.checked_add(take)?);
    }
    if filled <= Decimal::ZERO || filled < qty || mid <= Decimal::ZERO {
        return None;
    }
    let vwap = cost.checked_div(filled)?;
    let past = if buy { vwap.checked_sub(mid)? } else { mid.checked_sub(vwap)? };
    Some(past.checked_div(mid)?.checked_mul(Decimal::from(10_000))?.max(Decimal::ZERO))
}

/// Median of one venue's settled `rate_8h` over the last `window_hours` up to `now_ms`: one
/// sample per settled hour (the first seen wins), hours in `(newest - window, newest]`, `None`
/// below 3/4 coverage or when the middle pair's sum overflows `Decimal`. One spike among a
/// window of floor settlements leaves it at the floor, where the mean would chase the spike.
pub fn trailing_median(
    samples: &[FundingSample],
    now_ms: i64,
    window_hours: u64,
) -> Option<Decimal> {
    let window = i64::try_from(window_hours).ok()?;
    let newest = now_ms.div_euclid(HOUR_MS);
    let mut seen = std::collections::HashSet::new();
    let mut rates: Vec<Decimal> = samples
        .iter()
        .filter_map(|s| {
            let h = s.time_ms.div_euclid(HOUR_MS);
            let inside = h > newest.saturating_sub(window) && h <= newest;
            (inside && seen.insert(h)).then_some(s.rate_8h)
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

#[cfg(test)]
#[path = "funding_tests.rs"]
mod tests;
