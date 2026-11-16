use bb_strategy_sdk::param_spec::{Loosening, ParamSpec, Range, Tier, whole};
use bb_strategy_sdk::{OrderType, Setting, ValidateConfig};
use rust_decimal::Decimal;
use serde::Deserialize;

/// Configuration for the Avellaneda-Stoikov market-making strategy.
///
/// Parameter intuition:
/// - `gamma` — risk aversion. Higher = more aggressive inventory skew (reservation price moves
///   further from mid as inventory grows).
/// - `kappa` — order-flow intensity. Higher = more competitive book, tighter optimal spread.
///   Hummingbot calibrates this from recent trade arrivals; we expose it directly for simplicity.
/// - `order_horizon_secs` (τ) — the finite horizon in the A-S formula. In crypto perps there's no
///   natural T; treat this as a tuning knob that scales the inventory-skew term. 60–300s is a
///   reasonable starting range.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvellanedaStoikovConfig {
    /// Broker to trade against — name matches the one passed to
    /// `HarnessBuilder::wire_broker` (e.g. `"bullet"`).
    #[serde(default = "default_exchange")]
    pub exchange: String,

    /// Trading symbol (e.g. "BTC-USD").
    pub symbol: String,

    /// Optional reference exchange whose mid drives `s` in the A-S formula
    /// instead of the trading venue's own mid. Only `"binance"` is wired up
    /// today. When unset, the trading venue's `BookUpdate` drives `s` (textbook
    /// single-venue A-S). Setting this turns the strategy into a fair-value
    /// MM: quotes are anchored to the reference and the local book is used
    /// only for inventory tracking and would-cross checks.
    #[serde(default)]
    pub reference_exchange: Option<String>,

    /// Symbol on the reference venue (e.g. `"btcusdt"` for Binance).
    /// Required when `reference_exchange` is set.
    #[serde(default)]
    pub reference_symbol: Option<String>,

    /// Which Binance venue to use as reference: `"perp"` (default,
    /// fstream.binance.com) or `"spot"`. Match perps-to-perps when trading
    /// against a perp DEX to avoid spot/funding-basis confounds.
    #[serde(default = "default_reference_market")]
    pub reference_market: String,

    /// If no reference update arrives within this many seconds, refuse to
    /// refresh quotes (we'd be quoting around a stale anchor).
    #[serde(default = "default_reference_stale_secs")]
    pub reference_stale_secs: u64,

    /// Order quantity per quote.
    pub order_size: Decimal,

    /// Hard position cap. Used both to normalize inventory for the skew
    /// calculation and as a safety gate on the leaned side.
    pub max_position: Decimal,

    /// Optional target inventory in base units (default 0 = neutral).
    #[serde(default)]
    pub inventory_target: Decimal,

    /// Risk aversion γ. Must be > 0.
    pub gamma: Decimal,

    /// Order-flow intensity κ. Must be > 0.
    pub kappa: Decimal,

    /// Finite horizon τ (seconds) used in the A-S closed form.
    #[serde(default = "default_order_horizon_secs")]
    pub order_horizon_secs: u64,

    /// Rolling window for the volatility estimator.
    #[serde(default = "default_vol_window_secs")]
    pub vol_window_secs: u64,

    /// Re-quote cadence. Quotes are refreshed on the first tick that hits this.
    #[serde(default = "default_order_refresh_secs")]
    pub order_refresh_secs: u64,

    /// Half-spread floor in basis points (bid/ask kept at least this far from
    /// reservation price). Prevents A-S from collapsing to 0 in low-vol
    /// regimes, which would quote inside fees.
    #[serde(default = "default_min_half_spread_bps")]
    pub min_half_spread_bps: Decimal,

    /// Half-spread ceiling in basis points. Caps A-S in vol spikes so we
    /// don't quote so wide that no fills are possible.
    #[serde(default = "default_max_half_spread_bps")]
    pub max_half_spread_bps: Decimal,

    /// Number of quote levels per side. `1` = classic textbook A-S (single
    /// bid/ask). Production MMs typically use 3–5. Outer levels are stepped
    /// outward from the A-S inner quote by `order_level_spread_bps`.
    #[serde(default = "default_order_levels")]
    pub order_levels: usize,

    /// Additional half-spread (in bps of mid) between consecutive levels.
    /// Level `i` sits at `reservation ± (inner_half_spread + i × step)`.
    /// Only used when `order_levels > 1`.
    #[serde(default = "default_order_level_spread_bps")]
    pub order_level_spread_bps: Decimal,

    /// Per-level size multiplier increment. Level `i` size is
    /// `order_size × (1 + i × amount_step)`. `0` = flat size across levels.
    #[serde(default)]
    pub order_level_amount_step: Decimal,

    /// Order type for quotes. TOML: `"post_only"` (default), `"limit"`, `"market"`.
    #[serde(default = "default_order_type")]
    pub order_type: OrderType,

    /// Base amend threshold (in bps of mid) for the inner level. A slot's
    /// new target price must drift by at least this much before we amend.
    /// `0` amends on every refresh.
    #[serde(default = "default_amend_threshold_bps")]
    pub amend_threshold_bps: Decimal,

    /// Per-level threshold widening. Level `i`'s threshold is
    /// `amend_threshold_bps + i × amend_threshold_step_bps`. Outer levels
    /// fill less often, so chasing fair value on them just burns API
    /// round-trips — widening their threshold cuts churn without hurting
    /// fill quality. `0` (default) means flat across levels.
    #[serde(default)]
    pub amend_threshold_step_bps: Decimal,

    /// Minimum interval between API-emitting refreshes. Reference feeds
    /// (e.g. Binance bookTicker) can fire faster than Bullet REST round-trips
    /// complete; without this throttle, events queue up in the actor's input
    /// channel and get dropped, producing "actor lagged" warnings. The
    /// dropped events are benign (we always quote off the latest mid), but
    /// the throttle keeps the event loop responsive. Default 100ms — set to
    /// 0 to disable.
    #[serde(default = "default_min_refresh_interval_ms")]
    pub min_refresh_interval_ms: u64,

    /// Periodic safety-net reconciliation interval (seconds). The
    /// authoritative source of slot state is the WS user-data stream
    /// (`OrderLifecycle` + `Trade` events from Bullet's
    /// `ADDRESS@ORDER_TRADE_UPDATE`); REST `get_open_orders` is queried only
    /// on this interval to recover from any dropped WS frames. Drops
    /// phantom slots (tracked locally, gone on Bullet) and cancels orphan
    /// orders (alive on Bullet, untracked locally). `0` disables the
    /// safety net entirely (pure WS-first).
    #[serde(default = "default_reconcile_interval_secs")]
    pub reconcile_interval_secs: u64,

    /// Optional fee-floor guardrail. If set, `on_start` refuses to run when
    /// the configured `min_half_spread_bps` can't cover round-trip maker fees.
    #[serde(default)]
    pub fees: Option<AsFees>,
}

/// Fee-floor guardrail for the A-S strategy.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsFees {
    pub maker_bps: Decimal,
    #[serde(default)]
    pub taker_bps: Decimal,
    /// `min_half_spread_bps × 2` must be at least `multiplier × 2 × maker_bps`.
    #[serde(default = "default_min_spread_fee_multiplier")]
    pub min_spread_fee_multiplier: Decimal,
}

fn default_order_horizon_secs() -> u64 {
    60
}

fn default_vol_window_secs() -> u64 {
    600
}

fn default_order_refresh_secs() -> u64 {
    10
}

fn default_min_half_spread_bps() -> Decimal {
    Decimal::from(5)
}

fn default_max_half_spread_bps() -> Decimal {
    Decimal::from(500)
}

fn default_order_type() -> OrderType {
    OrderType::PostOnly
}

fn default_exchange() -> String {
    "bullet".to_string()
}

fn default_min_spread_fee_multiplier() -> Decimal {
    Decimal::from(3)
}

fn default_order_levels() -> usize {
    3
}

fn default_order_level_spread_bps() -> Decimal {
    Decimal::from(10)
}

fn default_amend_threshold_bps() -> Decimal {
    Decimal::new(5, 1)
}

fn default_min_refresh_interval_ms() -> u64 {
    100
}

fn default_reconcile_interval_secs() -> u64 {
    10
}

fn default_reference_market() -> String {
    "perp".to_string()
}

fn default_reference_stale_secs() -> u64 {
    10
}

impl ValidateConfig for AvellanedaStoikovConfig {
    fn validate(&self) -> Result<(), String> {
        if self.exchange != "bullet" {
            return Err(format!(
                "exchange {:?}: the strategy trades on Bullet only, so exchange must be \"bullet\"",
                self.exchange
            ));
        }
        if self.order_size <= Decimal::ZERO {
            return Err("order_size must be positive".into());
        }
        if self.max_position < self.order_size {
            return Err("max_position must be >= order_size".into());
        }
        if self.gamma <= Decimal::ZERO {
            return Err("gamma must be positive".into());
        }
        if self.kappa <= Decimal::ZERO {
            return Err("kappa must be positive".into());
        }
        if self.order_horizon_secs == 0 {
            return Err("order_horizon_secs must be >= 1".into());
        }
        if self.vol_window_secs == 0 {
            return Err("vol_window_secs must be >= 1".into());
        }
        if self.min_half_spread_bps <= Decimal::ZERO {
            return Err("min_half_spread_bps must be positive".into());
        }
        if self.max_half_spread_bps < self.min_half_spread_bps {
            return Err("max_half_spread_bps must be >= min_half_spread_bps".into());
        }
        if self.order_levels == 0 {
            return Err("order_levels must be >= 1".into());
        }
        if self.order_level_spread_bps < Decimal::ZERO {
            return Err("order_level_spread_bps must be non-negative".into());
        }
        if self.order_level_amount_step < Decimal::ZERO {
            return Err("order_level_amount_step must be non-negative".into());
        }
        if self.amend_threshold_bps < Decimal::ZERO {
            return Err("amend_threshold_bps must be non-negative".into());
        }
        if self.amend_threshold_step_bps < Decimal::ZERO {
            return Err("amend_threshold_step_bps must be non-negative".into());
        }
        if self.reference_exchange.is_some() && self.reference_symbol.is_none() {
            return Err("reference_exchange requires reference_symbol".into());
        }
        if self.reference_stale_secs == 0 {
            return Err("reference_stale_secs must be >= 1".into());
        }
        if let Some(fees) = &self.fees {
            if fees.maker_bps < Decimal::ZERO {
                return Err("fees.maker_bps must be non-negative".into());
            }
            if fees.min_spread_fee_multiplier < Decimal::ONE {
                return Err("fees.min_spread_fee_multiplier must be >= 1".into());
            }
        }
        Ok(())
    }
}

impl AvellanedaStoikovConfig {
    pub fn validate(&self) -> Result<(), String> {
        <Self as ValidateConfig>::validate(self)
    }
}

const ORDER_TYPES: &[&str] = &["limit", "post_only", "market"];

fn structural(spec: ParamSpec) -> Setting {
    Setting::of(spec.tier(Tier::Structural))
}

/// Every `[strategies.avellaneda-stoikov]` setting.
#[must_use]
#[expect(clippy::too_many_lines, reason = "one declaration per setting")]
pub fn settings() -> Vec<Setting> {
    let any = Range::RUNTIME;
    // A guard's guarded range ends at its default on the loose side: the agent tightens within
    // it, and only the owner signs past it (security agent-surface-M3).
    let at_least = |n| Range { min: Some(whole(n)), max: None };
    // A model input has a sane hard range that refuses an absurd value, and no signature tier:
    // its guarded range is the same.
    let secs = |name, default, range: Range, help| {
        structural(ParamSpec::int(name, default, range, range, help))
    };
    let model = |name, default, range: Range, help| {
        structural(ParamSpec::decimal(name, default, range, range, help))
    };
    let positive = |max| Range::new(Decimal::new(1, 4), whole(max));
    vec![
        structural(ParamSpec::text("exchange", "bullet", "The registered exchange it trades on.")),
        structural(ParamSpec::text("symbol", "BTC-USD", "Trading symbol.")).required(),
        structural(ParamSpec::text(
            "reference_exchange",
            "binance",
            "Exchange whose mid prices the quotes (only binance); omit to use the trading venue's.",
        )),
        structural(ParamSpec::text(
            "reference_symbol",
            "btcusdt",
            "Symbol on the reference venue; needed with reference_exchange.",
        )),
        structural(ParamSpec::choice(
            "reference_market",
            &["perp", "spot"],
            0,
            "Binance market used as the reference.",
        )),
        structural(
            ParamSpec::int(
                "reference_stale_secs",
                10,
                Range::whole(1, 10),
                any,
                "Stop requoting when the reference is older than this.",
            )
            .loosening(Loosening::Up),
        ),
        Setting::of(
            ParamSpec::decimal("order_size", whole(1), any, any, "Quantity per quote.")
                .tier(Tier::Limit)
                .loosening(Loosening::Up),
        )
        .required(),
        Setting::of(
            ParamSpec::decimal("max_position", whole(1), any, any, "Hard position cap.")
                .tier(Tier::Limit)
                .loosening(Loosening::Up),
        )
        .required(),
        model(
            "inventory_target",
            whole(0),
            Range::new(Decimal::new(-1_000_000, 0), whole(1_000_000)),
            "Target inventory in base units.",
        ),
        model("gamma", whole(1), positive(100), "Risk aversion γ (> 0).").required(),
        model("kappa", whole(1), positive(10_000), "Order-flow intensity κ (> 0).").required(),
        secs("order_horizon_secs", 60, Range::whole(1, 86_400), "Horizon τ in the closed form (s)."),
        secs("vol_window_secs", 600, Range::whole(1, 86_400), "Volatility estimator window (s)."),
        // A cadence that keeps the quotes near the market has the market maker's bounds.
        structural(
            ParamSpec::int(
                "order_refresh_secs",
                10,
                Range::whole(1, 60),
                any,
                "Requote cadence (s).",
            )
            .loosening(Loosening::Up),
        ),
        structural(
            ParamSpec::decimal(
                "min_half_spread_bps",
                whole(5),
                at_least(5),
                any,
                "Half-spread floor (bps).",
            )
            .loosening(Loosening::Down),
        ),
        model("max_half_spread_bps", whole(500), Range::whole(1, 10_000), "Half-spread ceiling (bps)."),
        Setting::of(
            ParamSpec::int("order_levels", 3, any, any, "Quote levels per side.")
                .tier(Tier::Limit)
                .loosening(Loosening::Up),
        ),
        model(
            "order_level_spread_bps",
            whole(10),
            Range::whole(0, 1_000),
            "Extra half-spread between levels (bps).",
        ),
        model(
            "order_level_amount_step",
            whole(0),
            Range::whole(0, 10),
            "Per-level size multiplier increment.",
        ),
        structural(ParamSpec::choice("order_type", ORDER_TYPES, 1, "Order type for quotes.")),
        structural(
            ParamSpec::decimal(
                "amend_threshold_bps",
                Decimal::new(5, 1),
                Range::whole(0, 50),
                any,
                "Drift before the inner level is amended (bps).",
            )
            .loosening(Loosening::Up),
        ),
        model(
            "amend_threshold_step_bps",
            whole(0),
            Range::whole(0, 100),
            "Per-level widening of the amend threshold (bps).",
        ),
        structural(
            ParamSpec::int(
                "min_refresh_interval_ms",
                100,
                Range::whole(0, 1000),
                any,
                "Minimum gap between refreshes that call the API (ms).",
            )
            .loosening(Loosening::Up),
        ),
        structural(
            ParamSpec::int(
                "reconcile_interval_secs",
                10,
                Range::whole(1, 10),
                any,
                "REST reconcile interval (s); 0 disables it.",
            )
            .loosening(Loosening::Up),
        ),
        Setting::under(
            "fees",
            // The fee floor is built from it: lowering a fee the owner set loosens the floor.
            ParamSpec::decimal(
                "maker_bps",
                whole(0),
                Range::new(whole(0), whole(1000)),
                Range::new(whole(0), whole(1000)),
                "Maker fee (bps); the fee floor is built from it, so lowering it loosens the floor.",
            )
            .tier(Tier::Structural)
            .loosening(Loosening::Down),
        ),
        Setting::under(
            "fees",
            ParamSpec::decimal(
                "taker_bps",
                whole(0),
                Range::whole(0, 1_000),
                Range::whole(0, 1_000),
                "Taker fee (bps).",
            )
                .tier(Tier::Structural),
        ),
        Setting::under(
            "fees",
            ParamSpec::decimal(
                "min_spread_fee_multiplier",
                whole(3),
                at_least(3),
                any,
                "min_half_spread_bps must cover this many round-trip maker fees.",
            )
            .tier(Tier::Structural)
            .loosening(Loosening::Down),
        ),
    ]
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
