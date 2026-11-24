use rust_decimal::Decimal;

use super::{DayOpen, DayPnl, DayStart, PersistedStops, utc_day};

const NOON: u64 = 1_761_566_400_000;
const DAY_MS: u64 = 86_400_000;
const MORNING: u64 = NOON - 6 * 3_600_000;

fn d(s: &str) -> Decimal {
    s.parse().unwrap_or_default()
}

/// What a restart loads: `stops` through the file's JSON.
fn reloaded(stops: &PersistedStops) -> PersistedStops {
    let json = serde_json::to_string(stops).unwrap_or_default();
    serde_json::from_str(&json).unwrap_or_default()
}

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

#[test]
fn a_restart_mid_day_keeps_the_mornings_value() {
    let mut open = DayOpen::default();
    assert_eq!(open.read(d("1000"), MORNING), Some(d("1000")), "go-live: the first read");
    assert_eq!(open.read(d("900"), NOON), Some(d("1000")), "a loss doesn't move it");
    let saved = PersistedStops {
        day_pnl_day: Some(utc_day(NOON)),
        day_pnl_carry: Some(d("-100")),
        day_start: open.to_persist(),
        ..PersistedStops::default()
    };

    let mut again = DayOpen::load(Some(&reloaded(&saved)));
    assert_eq!(again.read(d("1500"), NOON + 1), Some(d("1000")));
    assert_eq!(again.to_persist(), saved.day_start, "a save writes the morning's value back");
}

#[test]
fn a_deposit_the_same_day_doesnt_raise_it() {
    let mut open = DayOpen::default();
    open.read(d("1000"), MORNING);
    assert_eq!(open.read(d("6000"), NOON), Some(d("1000")));
}

#[test]
fn a_new_day_records_its_first_read() {
    let mut open = DayOpen::default();
    open.read(d("1000"), NOON);
    assert_eq!(open.read(d("1200"), NOON + DAY_MS), Some(d("1200")));
    assert_eq!(open.read(d("1300"), NOON + DAY_MS + 1), Some(d("1200")));

    let yesterday = PersistedStops {
        day_pnl_day: Some(utc_day(NOON)),
        day_start: Some(DayStart { day: utc_day(NOON), value: d("1000") }),
        ..PersistedStops::default()
    };
    let mut restarted = DayOpen::load(Some(&reloaded(&yesterday)));
    assert_eq!(restarted.read(d("700"), NOON + DAY_MS), Some(d("700")), "a restart the next day");
}

#[test]
fn a_clock_stepping_back_keeps_the_days_value() {
    let mut open = DayOpen::default();
    open.read(d("1200"), NOON + DAY_MS);
    assert_eq!(open.read(d("5000"), NOON), Some(d("1200")));
}

/// A file an older version wrote today has the day's P&L but not its start: the account's value
/// now may count a deposit made earlier today, which would raise the stop. The start stays
/// unknown (no new orders) until the next day's first read records one.
#[test]
fn an_older_file_from_today_loads_with_the_start_unknown_until_the_next_day() {
    let old = r#"{"daily_loss_day":null,"paused":false,"day_pnl_day":"2025-10-27","day_pnl_carry":"-5","day_equity":"6000"}"#;
    let loaded: PersistedStops = serde_json::from_str(old).unwrap_or_default();
    assert_eq!(loaded.day_pnl_day, Some(utc_day(NOON)), "it loads");
    assert_eq!(loaded.day_start, None);

    let mut open = DayOpen::load(Some(&loaded));
    assert_eq!(open.read(d("6000"), NOON), None);
    assert_eq!(open.to_persist(), None);
    assert_eq!(DayOpen::load(Some(&reloaded(&loaded))).read(d("6000"), NOON + 1), None);
    assert_eq!(open.read(d("6100"), NOON + DAY_MS), Some(d("6100")), "the next day's first read");
}

#[test]
fn an_older_file_from_an_earlier_day_records_the_first_read() {
    let old =
        r#"{"daily_loss_day":null,"paused":false,"day_pnl_day":"2025-10-26","day_pnl_carry":"-5"}"#;
    let loaded: PersistedStops = serde_json::from_str(old).unwrap_or_default();
    assert_eq!(DayOpen::load(Some(&loaded)).read(d("800"), NOON), Some(d("800")));
    assert_eq!(DayOpen::load(None).read(d("800"), NOON), Some(d("800")), "nothing saved");
}

/// The saved format: `day_start` is written only when set, so a file without one is unchanged.
#[test]
fn golden_the_day_start_in_the_saved_stops() {
    let stops = PersistedStops {
        day_pnl_day: Some(utc_day(NOON)),
        day_start: Some(DayStart { day: utc_day(NOON), value: d("1000.5") }),
        ..PersistedStops::default()
    };
    assert_eq!(
        serde_json::to_string(&stops).unwrap_or_default(),
        r#"{"daily_loss_day":null,"paused":false,"day_pnl_day":"2025-10-27","day_pnl_carry":null,"day_start":{"day":"2025-10-27","value":"1000.5"}}"#
    );
    assert_eq!(
        serde_json::to_string(&PersistedStops::default()).unwrap_or_default(),
        r#"{"daily_loss_day":null,"paused":false,"day_pnl_day":null,"day_pnl_carry":null}"#
    );
}
