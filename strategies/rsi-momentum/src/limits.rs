//! Fixed safety constants of the strategy: not config, not tunable.

pub use bb_strategy_support::ioc::MAX_IOC_BPS_FROM_MID;

/// Closes cross this many times `slippage_bps`, times the attempt number.
pub const CLOSE_SLIPPAGE_MULT: u32 = 3;
/// Close attempts (one per tick) before the owner is asked; the closes go on every tick after.
pub const CLOSE_ATTEMPTS: u32 = 5;
/// An entry the venue took (or may have) blocks the next one until its fill arrives or this long
/// has passed (the venue read then settles it).
pub const ENTRY_SETTLE_SECS: u64 = 10;
