//! How a [`Decimal`] crosses the `bb:strategy` world: its 96-bit mantissa as a signed 128-bit
//! integer split into `hi` (signed, the top 64 bits) and `lo` (the bottom 64 bits), plus its scale.
//! value = (hi * 2^64 + lo) / 10^scale. Exact, no parsing, fixed size. A negative zero crosses as
//! zero (they compare equal).

use rust_decimal::Decimal;

/// The wire form of a [`Decimal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireDecimal {
    pub hi: i64,
    pub lo: u64,
    pub scale: u8,
}

/// A wire decimal that is no [`Decimal`]: a mantissa past 96 bits or a scale past 28.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadDecimal(pub WireDecimal);

impl std::fmt::Display for BadDecimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let WireDecimal { hi, lo, scale } = self.0;
        write!(f, "not a decimal: hi {hi}, lo {lo}, scale {scale}")
    }
}

impl std::error::Error for BadDecimal {}

const LOW_BITS: i128 = 0xFFFF_FFFF_FFFF_FFFF;

#[must_use]
pub fn encode(value: Decimal) -> WireDecimal {
    let mantissa = value.mantissa();
    // |mantissa| < 2^96, so its top half fits an i64; the low half is 64 bits by the mask.
    let hi = i64::try_from(mantissa >> 64).unwrap_or(0);
    let lo = u64::try_from(mantissa & LOW_BITS).unwrap_or(0);
    // A Decimal's scale is at most 28.
    let scale = u8::try_from(value.scale()).unwrap_or(u8::MAX);
    WireDecimal { hi, lo, scale }
}

/// # Errors
/// [`BadDecimal`] when the mantissa is past 96 bits or the scale past 28.
pub fn decode(wire: WireDecimal) -> Result<Decimal, BadDecimal> {
    let mantissa = (i128::from(wire.hi) << 64) | i128::from(wire.lo);
    Decimal::try_from_i128_with_scale(mantissa, u32::from(wire.scale)).map_err(|_| BadDecimal(wire))
}

#[cfg(test)]
#[path = "decimal_tests.rs"]
mod tests;
