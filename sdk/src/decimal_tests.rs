use rust_decimal::Decimal;

use super::*;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

#[test]
fn every_kind_of_decimal_round_trips_exactly() {
    let values = [
        Decimal::ZERO,
        Decimal::ONE,
        Decimal::NEGATIVE_ONE,
        d("75.5"),
        d("-0.000000000000000000000000001"),
        d("0.0000000000000000000000000001"),
        d("123456789.123456789"),
        d("-98765.4321"),
        d("18446744073709551616"),
        d("-18446744073709551617"),
        Decimal::MAX,
        Decimal::MIN,
        Decimal::new(1, 28),
        Decimal::new(-1, 28),
    ];
    for value in values {
        let wire = encode(value);
        let back = decode(wire).unwrap();
        assert_eq!(back, value, "{value} crossed as {wire:?}");
        assert_eq!(back.scale(), value.scale(), "{value} keeps its scale");
    }
}

#[test]
fn the_wire_form_is_the_mantissa_split_in_two_and_the_scale() {
    assert_eq!(encode(d("75.5")), WireDecimal { hi: 0, lo: 755, scale: 1 });
    assert_eq!(encode(d("-1")), WireDecimal { hi: -1, lo: u64::MAX, scale: 0 });
    // 2^64 = hi 1, lo 0.
    assert_eq!(encode(d("18446744073709551616")), WireDecimal { hi: 1, lo: 0, scale: 0 });
}

#[test]
fn a_negative_zero_crosses_as_zero() {
    let neg_zero = Decimal::new(-0, 2).set_sign_negative_copy();
    assert_eq!(decode(encode(neg_zero)).unwrap(), Decimal::ZERO);
}

#[test]
fn a_mantissa_past_96_bits_or_a_scale_past_28_is_refused() {
    let too_wide = WireDecimal { hi: 1 << 33, lo: 0, scale: 0 };
    assert_eq!(decode(too_wide), Err(BadDecimal(too_wide)));
    let too_fine = WireDecimal { hi: 0, lo: 1, scale: 29 };
    assert_eq!(decode(too_fine), Err(BadDecimal(too_fine)));
}

trait NegCopy {
    fn set_sign_negative_copy(self) -> Self;
}

impl NegCopy for Decimal {
    fn set_sign_negative_copy(mut self) -> Self {
        self.set_sign_negative(true);
        self
    }
}

/// The edges of the 96-bit mantissa: 2^96 - 1 (`Decimal::MAX`) crosses, ±2^96 and `i64::MIN` in the
/// top half do not.
#[test]
fn the_mantissa_edges_are_exact() {
    let max = WireDecimal { hi: (1 << 32) - 1, lo: u64::MAX, scale: 0 };
    assert_eq!(decode(max), Ok(Decimal::MAX));
    assert_eq!(encode(Decimal::MAX), max);
    let min = WireDecimal { hi: -(1 << 32), lo: 1, scale: 0 };
    assert_eq!(decode(min), Ok(Decimal::MIN), "-(2^96 - 1)");
    for refused in [
        WireDecimal { hi: 1 << 32, lo: 0, scale: 0 },
        WireDecimal { hi: -(1 << 32), lo: 0, scale: 0 },
        WireDecimal { hi: i64::MIN, lo: 0, scale: 0 },
        WireDecimal { hi: i64::MIN, lo: u64::MAX, scale: 28 },
    ] {
        assert_eq!(decode(refused), Err(BadDecimal(refused)), "{refused:?}");
    }
}
