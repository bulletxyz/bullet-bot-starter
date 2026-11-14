//! Fixed safety constants of the strategy: not config, not tunable.

use rust_decimal::Decimal;

/// No IOC is priced further than this from the book's mid (bps; the mark when no live book),
/// however far a close retry widens it.
pub const MAX_IOC_BPS_FROM_MID: Decimal = Decimal::from_parts(200, 0, 0, false, 0);
/// Closes cross this many times `slippage_bps`, times the attempt number.
pub const CLOSE_SLIPPAGE_MULT: u32 = 3;
/// Close attempts (one per tick) before the owner is asked; the closes go on every tick after.
pub const CLOSE_ATTEMPTS: u32 = 5;
/// An entry the venue took (or may have) blocks the next one until its fill arrives or this long
/// has passed (the venue read then settles it).
pub const ENTRY_SETTLE_SECS: u64 = 10;
