//! Fixed safety constants of the execution actor: not config, not tunable.

use rust_decimal::Decimal;

/// Book levels read for an order's touch and for depth.
pub const BOOK_DEPTH: usize = 20;
/// Settled funding is re-read this often (it settles hourly).
pub const SIGNAL_REFRESH_SECS: u64 = 600;
/// A failed read is retried after this.
pub const SIGNAL_RETRY_SECS: u64 = 60;
/// A signal not confirmed for this long is dropped: unknown blocks entries, not holds.
pub const SIGNAL_MAX_AGE_SECS: u64 = 7_200;
/// Funding histories fetched per tick (held symbols first).
pub const FUNDING_FETCHES_PER_TICK: usize = 6;
/// Taker fees are re-read this often.
pub const FEE_REFRESH_SECS: u64 = 3_600;
/// Unknown fees are retried after this.
pub const FEE_RETRY_SECS: u64 = 60;
/// Open interest is re-read this often.
pub const OI_REFRESH_SECS: u64 = 60;
/// Open-interest reads per tick.
pub const OI_FETCHES_PER_TICK: usize = 8;
/// A funding rate beyond this per 8h (5%) is bad data: no signal. Real Bullet spikes reach 3%.
pub const MAX_FUNDING_RATE_8H: Decimal = Decimal::from_parts(5, 0, 0, false, 2);
/// New positions per side per tick.
pub const ENTRIES_PER_SIDE_PER_TICK: usize = 1;
/// A top-up smaller than this share of the target is not worth an order.
pub const MIN_GROW_SHARE: Decimal = Decimal::from_parts(1, 0, 0, false, 1);
/// A held leg below this share of its target is topped up.
pub const TOP_UP_BELOW: Decimal = Decimal::from_parts(9, 0, 0, false, 1);
/// A leg must be at least this share of `notional_per_symbol` to be worth entering.
pub const MIN_ENTRY_SHARE: Decimal = Decimal::from_parts(5, 0, 0, false, 1);
/// No IOC is priced further than this from the book's mid (bps), however far a flatten retry
/// widens it: the cap on what a close or an entry may give away.
pub const MAX_IOC_BPS_FROM_MID: Decimal = Decimal::from_parts(200, 0, 0, false, 0);
/// Flatten orders cross this many times `slippage_bps`, more on each retry.
pub const FLATTEN_SLIPPAGE_MULT: u32 = 3;
/// Flatten attempts before giving up (the failure is returned).
pub const FLATTEN_ATTEMPTS: u32 = 5;
/// Wait between flatten attempts, ms (the bot's clock).
pub const FLATTEN_RETRY_WAIT_MS: u64 = 1_000;
/// The hurdle is exit-hysteresis: exit when carry falls under this share of the cost.
pub const EXIT_HURDLE_SHARE: Decimal = Decimal::from_parts(5, 0, 0, false, 1);
/// Open interest not refreshed for this long is dropped: unknown blocks entries.
pub const OI_MAX_AGE_SECS: u64 = 600;
/// Maintenance margin / free collateral at which everything is flattened and the bot halts.
/// Liquidation is at 1.
pub const LIQ_RATIO_FLATTEN: Decimal = Decimal::from_parts(8, 0, 0, false, 1);
/// Consecutive ticks the long and short books may sit past `max_net_pct` before the
/// lowest-ranked leg of the heavy side is exited.
pub const IMBALANCE_TICKS: u32 = 5;
/// An order whose submit was ambiguous (timeout, no answer) is reconciled from the venue's
/// position only after this long: a commit that is still in flight has landed by then.
pub const AMBIGUOUS_SETTLE_SECS: u64 = 5;
/// The first retry of a failed unwind waits this long; each failure doubles the wait.
pub const UNWIND_BACKOFF_BASE_SECS: u64 = 2;
/// The longest wait between unwind retries.
pub const UNWIND_BACKOFF_MAX_SECS: u64 = 60;
/// Failed unwind attempts after which the alert says the bot is stuck with a one-sided book.
pub const UNWIND_ESCALATE_AFTER: u32 = 5;
/// A pair's second leg may fill this share short of the first before the first is trimmed.
pub const PAIR_TOLERANCE: Decimal = Decimal::from_parts(1, 0, 0, false, 1);
