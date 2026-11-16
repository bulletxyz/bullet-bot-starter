//! Every setting of the section, with its ranges: the params a live change may set
//! ([`BUILTIN_SPECS`], the strategy's own [`PARAMS`] under `extra`), the limits and the structural
//! fields.

use bb_strategy_sdk::param_spec::{Loosening, ParamSpec, Range, Tier, whole};
use bb_strategy_sdk::{Decimal, HostLimit, Setting};

use super::DEFAULT_MAX_ORDERS_PER_MIN;

const fn dec(n: u32, scale: u32) -> Decimal {
    Decimal::from_parts(n, 0, 0, false, scale)
}

/// The params a live change may set, beside [`PARAMS`]. `order_size`'s range is the venue's step
/// to half the position cap, resolved at runtime.
pub const BUILTIN_SPECS: &[ParamSpec] = &[
    ParamSpec::decimal(
        "bid_spread_bps",
        dec(10, 0),
        Range::whole(2, 200),
        // Down to 0.01 bps with the owner's signature: as tight as the touch if they choose.
        Range::new(dec(1, 2), dec(1000, 0)),
        "Bid distance below the fair value.",
    )
    .loosening(Loosening::Down),
    ParamSpec::decimal(
        "ask_spread_bps",
        dec(10, 0),
        Range::whole(2, 200),
        Range::new(dec(1, 2), dec(1000, 0)),
        "Ask distance above the fair value.",
    )
    .loosening(Loosening::Down),
    ParamSpec::decimal(
        "order_size",
        dec(1, 4),
        Range::RUNTIME,
        Range::RUNTIME,
        "Size of each quote: the venue step to half the max position.",
    )
    .loosening(Loosening::Up),
    ParamSpec::int(
        "refresh_secs",
        5,
        Range::whole(1, 60),
        Range::whole(1, 600),
        "Cancel/replace cadence.",
    )
    .loosening(Loosening::Up),
    ParamSpec::decimal(
        "refresh_threshold_bps",
        dec(1, 0),
        Range::new(dec(5, 1), dec(50, 0)),
        Range::new(dec(1, 1), dec(200, 0)),
        "Drift from the desired price before a quote is replaced.",
    )
    .loosening(Loosening::Up),
    ParamSpec::decimal(
        "skew_bps",
        dec(0, 0),
        Range::whole(0, 100),
        Range::whole(0, 500),
        "Inventory skew at max position.",
    )
    .loosening(Loosening::Down),
    ParamSpec::choice("fair_value", &["binance", "bullet"], 0, "Where the fair value comes from."),
    ParamSpec::int(
        "basis_halflife_secs",
        300,
        Range::whole(10, 3600),
        Range::whole(5, 86_400),
        "Half-life of the Bullet-Binance basis EMA.",
    ),
    ParamSpec::int(
        "reference_stale_secs",
        5,
        Range::whole(1, 10),
        Range::whole(1, 60),
        "Binance silence before the Bullet-mid fallback.",
    )
    .loosening(Loosening::Up),
    ParamSpec::decimal(
        "max_divergence_bps",
        dec(20, 0),
        Range::whole(5, 500),
        Range::whole(1, 2000),
        "Largest Binance fair value gap to the Bullet mid before the fallback.",
    )
    .loosening(Loosening::Up),
];

/// The volatility-widen switch: while the Bullet mid's high-low range within `vol_window_secs`
/// reaches `vol_threshold_bps`, both spreads widen by `vol_widen_bps`. Off by default, which
/// quotes exactly as without it.
pub const VOL_WIDEN_ENABLED: &str = "vol_widen_enabled";
pub const VOL_WINDOW_SECS: &str = "vol_window_secs";
pub const VOL_THRESHOLD_BPS: &str = "vol_threshold_bps";
pub const VOL_WIDEN_BPS: &str = "vol_widen_bps";

/// The strategy's own params, set under `extra`.
pub const PARAMS: &[ParamSpec] = &[
    ParamSpec::bool(
        VOL_WIDEN_ENABLED,
        false,
        "1 widens both spreads by vol_widen_bps while the Bullet mid's high-low range within \
         vol_window_secs is at least vol_threshold_bps; 0 (default) never widens.",
    )
    .loosening(Loosening::Down),
    ParamSpec::int(
        VOL_WINDOW_SECS,
        30,
        Range::whole(5, 300),
        Range::whole(1, 3600),
        "Seconds of Bullet mids the volatility range is measured over.",
    )
    .loosening(Loosening::Down),
    ParamSpec::decimal(
        VOL_THRESHOLD_BPS,
        whole(10),
        Range::whole(2, 200),
        Range::whole(1, 1000),
        "High-low range of the Bullet mid, in bps, that counts as volatile.",
    )
    .loosening(Loosening::Up),
    ParamSpec::decimal(
        VOL_WIDEN_BPS,
        whole(5),
        Range::whole(0, 100),
        Range::whole(0, 500),
        "Bps added to both spreads while volatile.",
    )
    .loosening(Loosening::Down),
];

const ORDER_TYPES: &[&str] = &["limit", "post_only", "market"];
/// The params a section must give (no default).
const REQUIRED_PARAMS: &[&str] = &["bid_spread_bps", "ask_spread_bps", "order_size"];

/// Every setting of the section: the structural fields, the limits, the fee floor and the params.
#[must_use]
pub fn settings() -> Vec<Setting> {
    let any = Range::RUNTIME;
    let structural = |spec: ParamSpec| Setting::of(spec.tier(Tier::Structural));
    let limit = |spec: ParamSpec| Setting::of(spec.tier(Tier::Limit));
    let fee =
        |spec: ParamSpec| Setting::under("fees", spec.tier(Tier::Guard).loosening(Loosening::Down));
    let mut out = vec![
        structural(ParamSpec::text("exchange", "bullet", "The registered exchange it trades on.")),
        structural(ParamSpec::text("symbol", "BTC-USD", "Trading symbol.")).required(),
        structural(ParamSpec::choice("order_type", ORDER_TYPES, 1, "Order type for quotes.")),
        structural(ParamSpec::text(
            "binance_symbol",
            "BTCUSDT",
            "Binance symbol for the fair value (default: base asset + USDT).",
        )),
        limit(ParamSpec::decimal("max_position", whole(1), any, any, "Position cap (base units)."))
            .required()
            .caps(HostLimit::MaxPosition),
        // No safe default: paper runs without a stop, but `bb setup` asks for one.
        limit(ParamSpec::decimal("daily_loss_usd", whole(25), any, any, "Daily loss stop (USD)."))
            .ask()
            .caps(HostLimit::DailyLossUsd),
        limit(ParamSpec::int(
            "stale_secs",
            30,
            any,
            any,
            "Pull quotes when the price is older (s).",
        )),
        limit(ParamSpec::int(
            "max_orders_per_min",
            DEFAULT_MAX_ORDERS_PER_MIN,
            any,
            any,
            "Order actions per minute.",
        )),
        limit(ParamSpec::decimal("max_notional_usd", whole(1000), any, any, "Notional cap (USD).")),
        fee(ParamSpec::decimal(
            "maker_bps",
            whole(0),
            Range::whole(0, 1000),
            Range::whole(0, 1000),
            "Maker fee (bps); the quote fee floor is built from it, so lowering it loosens the floor.",
        )),
        fee(ParamSpec::decimal(
            "min_spread_fee_multiplier",
            whole(3),
            Range::whole(1, 20),
            Range::whole(1, 20),
            "Spreads must cover this many round-trip maker fees; lowering it loosens the floor.",
        )),
    ];
    for spec in BUILTIN_SPECS {
        let s = Setting::of(*spec);
        let s = if REQUIRED_PARAMS.contains(&spec.name) { s.required() } else { s };
        // A size depends on the market: never filled in unasked.
        out.push(if spec.name == "order_size" { s.ask() } else { s });
    }
    out.extend(PARAMS.iter().map(|p| Setting::under("extra", *p)));
    out
}
