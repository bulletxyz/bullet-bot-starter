use rust_decimal::Decimal;

use super::DayPnl;

const NOON: u64 = 1_761_566_400_000;

/// A paper account's total can be any `Decimal` a plugin's fills add up to: the day's P&L past
/// one saturates, a gain up and a loss down (which trips the stop), never an overflow panic that
/// aborts the bot (security plugins-NV2).
#[test]
fn a_day_pnl_past_a_decimal_saturates_not_a_panic() {
    let mut gain = DayPnl::default();
    assert_eq!(gain.update(Decimal::MIN, NOON), Decimal::ZERO, "the day's first read");
    assert_eq!(gain.update(Decimal::MAX, NOON + 1), Decimal::MAX);
    let mut loss = DayPnl::default();
    loss.update(Decimal::MAX, NOON);
    assert_eq!(loss.update(Decimal::MIN, NOON + 1), Decimal::MIN);
}
