use rust_decimal::Decimal;

use super::*;
use crate::param_spec::{Range, Tier};

/// A cap is the bot's, not the descriptor's JSON: the owner's page and the goldens read the same
/// shape as before.
#[test]
fn a_setting_names_the_host_limit_it_sets_outside_its_json() {
    let spec =
        ParamSpec::decimal("max_position", Decimal::ONE, Range::RUNTIME, Range::RUNTIME, "Cap.")
            .tier(Tier::Limit);
    let plain = Setting::of(spec);
    assert_eq!(plain.caps, None);
    let capped = Setting::of(spec).caps(HostLimit::MaxPosition);
    assert_eq!(capped.caps, Some(HostLimit::MaxPosition));
    assert_eq!(serde_json::to_value(&capped).unwrap(), serde_json::to_value(&plain).unwrap());
    assert_eq!(
        Setting::under("limits", spec).caps(HostLimit::DailyLossUsd).caps,
        Some(HostLimit::DailyLossUsd)
    );
}
