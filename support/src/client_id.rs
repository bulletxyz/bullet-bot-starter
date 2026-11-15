//! Monotonic client-order-id issuer. Lives here instead of being re-invented
//! in every strategy. Format is decimal-encoded u64 so adapters that require
//! a u64 `ClientOrderId` (Bullet) can parse directly, while adapters that
//! accept arbitrary strings (HL via `cloid`) can use the same value verbatim.
//!
//! A copy of bb-core's `helpers::client_id` without `session_seeded` (it read the wall clock).
//!
//! The bot's own ids ([`ClientIdIssuer::own`], [`is_own`]) carry [`OWN_MARK`] in bits 45-52, under
//! 2^53: a later run tells the orders a bb run left resting from the owner's own orders on the
//! same market by the id alone, so its normal cancels take only those (an order whose outcome was
//! never known included). Ids of builds before the mark read as someone else's.

/// The mark in bits 45-52 of every id [`ClientIdIssuer::own`] hands out.
pub const OWN_MARK: u64 = 0xBB;
const SEQ_BITS: u32 = 45;
const SEQ_MASK: u64 = (1 << SEQ_BITS) - 1;

/// Whether `client_id` is one the bot issued ([`ClientIdIssuer::own`]): a `u64` with [`OWN_MARK`]
/// in bits 45-52 and nothing above.
#[must_use]
pub fn is_own(client_id: &str) -> bool {
    client_id.parse::<u64>().is_ok_and(|id| id >> SEQ_BITS == OWN_MARK)
}

#[derive(Debug, Clone, Default)]
pub struct ClientIdIssuer {
    next: u64,
}

impl ClientIdIssuer {
    pub fn new() -> Self {
        Self { next: 1 }
    }

    /// Start the sequence past a known high-water mark. The next `issue()`
    /// returns `start.max(1)`. Useful for crash recovery: on restart,
    /// resume past the last `client_id` the venue might still have live so
    /// fresh orders don't collide with stale ones.
    pub fn starting_at(start: u64) -> Self {
        Self { next: start.max(1) }
    }

    /// The bot's own ids ([`is_own`]), seeded from the bot clock `unix_ms`: ten ids per ms of wall
    /// time before a later start's range, until the year 2081.
    #[must_use]
    pub fn own(unix_ms: u64) -> Self {
        Self::starting_at(OWN_MARK << SEQ_BITS | (unix_ms.saturating_mul(10) & SEQ_MASK))
    }

    /// The next id. At `u64::MAX` it keeps returning `u64::MAX` (no wrap, no panic).
    /// Unreachable in practice: `own` starts near 1.8e13, so exhausting the
    /// range takes ~1.8e19 orders.
    pub fn issue(&mut self) -> String {
        let id = self.next.max(1);
        self.next = id.saturating_add(1);
        id.to_string()
    }
}

#[cfg(test)]
#[path = "client_id_tests.rs"]
mod tests;
