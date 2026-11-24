//! The market maker as the bot registers it: its name, its settings, its owner page, its presets,
//! the feeds it reads, and its build from its settings.

use std::collections::BTreeMap;

use bb_strategy_sdk::profile::{Amounts, PresetMarket, floor_to, money, pct, text, usable};
use bb_strategy_sdk::{
    About, Describe, Descriptor, ExchangeNeed, Featured, Feed, FromSettings, Needs, Preset,
    Profile, Risk, RiskLevel, SettingSpec, StrategyError, ValidateConfig,
};
use rust_decimal::{Decimal, RoundingStrategy};

use crate::{MarketMaker, MarketMakerSettings};

impl Describe for MarketMaker {
    /// The bot's name for its market maker since v1: its instances, config sections and log
    /// prefixes carry it.
    const NAME: &'static str = "simple-mm";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn describe() -> Descriptor {
        Descriptor {
            summary:
                "Market maker on one Bullet market: a bid and an ask around a fair value (the \
                      Binance mid plus the Bullet-Binance basis, or the Bullet mid), skewed by \
                      inventory, capped at the position limit."
                    .into(),
            settings: crate::settings::settings().iter().map(SettingSpec::from).collect(),
            settings_schema: schemars::schema_for!(MarketMakerSettings),
            legs: Vec::new(),
        }
    }

    fn profile() -> Option<Profile> {
        let spread = "bps. Wider earns more per trade, fills less";
        Some(Profile {
            about: About {
                title: "Market maker",
                tagline: "Earns the gap on many small trades",
                blurb: "Buys a little below the price and sells a little above it, all day. Earns \
                        the gap on many small trades.",
                risk: Risk {
                    level: RiskLevel::LowMedium,
                    text: "A sharp move can leave it holding a small position; your daily stop \
                           caps a bad day.",
                },
                featured: vec![
                    Featured::new("bid_spread_bps", "Buy below the price", spread, "{} bps"),
                    Featured::new("ask_spread_bps", "Sell above the price", spread, "{} bps"),
                    Featured::new("refresh_secs", "Requote every", "Seconds", "every {} s"),
                ],
            },
            settings: crate::settings::settings(),
            supports_paper: true,
            has_loss_stop: true,
            exchanges: vec![
                ExchangeNeed {
                    role: "venue",
                    setting: Some("exchange"),
                    default: "bullet",
                    tradable: true,
                    required: true,
                },
                ExchangeNeed {
                    role: "reference",
                    setting: None,
                    default: "binance",
                    tradable: false,
                    required: false,
                },
            ],
            presets: vec![BTC_CONSERVATIVE, ETH_BALANCED],
        })
    }

    /// The reference venue's prices for the fair value; the host's resync and wind-down as for
    /// any strategy.
    fn needs() -> Needs {
        Needs { feeds: vec![Feed::Reference], ..Needs::default() }
    }
}

impl FromSettings for MarketMaker {
    fn from_settings(settings: &str) -> Result<Self, StrategyError> {
        let settings: MarketMakerSettings = serde_json::from_str(settings)
            .map_err(|e| StrategyError(format!("simple-mm settings: {e}")))?;
        settings.validate().map_err(StrategyError)?;
        Ok(Self::new(settings))
    }
}

/// The price-staleness stop every preset sets (the floor needs one to go live).
const STALE_SECS: u32 = 30;

/// A market maker sized from the amount: both spreads at `spread_bps`, the position cap the
/// amount at the market's price and each quote `1/orders` of it, the day's loss stop `loss_pct`
/// percent of the amount.
#[derive(Debug, Clone, Copy)]
struct Recipe {
    title: &'static str,
    symbol: &'static str,
    spread_bps: u32,
    skew_bps: u32,
    orders: u32,
    loss_pct: u32,
}

fn size(
    r: &Recipe,
    amount: Decimal,
    market: Option<PresetMarket>,
) -> Result<BTreeMap<String, String>, String> {
    let m = usable(r.title, market)?;
    let cap = floor_to(amount.checked_div(m.price).unwrap_or_default(), m.step);
    let size = floor_to(cap.checked_div(Decimal::from(r.orders)).unwrap_or_default(), m.step);
    if size < m.step {
        let least = m
            .step
            .checked_mul(m.price)
            .and_then(|v| v.checked_mul(Decimal::from(r.orders)))
            .unwrap_or_default()
            .round_dp_with_strategy(0, RoundingStrategy::AwayFromZero);
        return Err(format!(
            "{} is too small for {} at {}: {} of its smallest orders take about {}",
            money(amount),
            r.symbol,
            money(m.price.round_dp(2)),
            r.orders,
            money(least)
        ));
    }
    Ok(BTreeMap::from([
        ("bid_spread_bps".into(), r.spread_bps.to_string()),
        ("ask_spread_bps".into(), r.spread_bps.to_string()),
        ("skew_bps".into(), r.skew_bps.to_string()),
        ("order_size".into(), text(size)),
        ("max_position".into(), text(cap)),
        ("max_notional_usd".into(), text(amount)),
        ("daily_loss_usd".into(), text(pct(amount, r.loss_pct))),
        ("stale_secs".into(), STALE_SECS.to_string()),
    ]))
}

fn worst_day(loss_pct: u32, amount: Decimal) -> String {
    format!(
        "Worst day: it stops itself after a {} loss (more only if the price jumps past the stop \
         before it can close).",
        money(pct(amount, loss_pct))
    )
}

const CONSERVATIVE: Recipe = Recipe {
    title: "Conservative BTC market maker",
    symbol: "BTC-USD",
    spread_bps: 30,
    skew_bps: 25,
    orders: 4,
    loss_pct: 5,
};

const BALANCED: Recipe = Recipe {
    title: "Balanced ETH market maker",
    symbol: "ETH-USD",
    spread_bps: 15,
    skew_bps: 15,
    orders: 4,
    loss_pct: 10,
};

const BTC_CONSERVATIVE: Preset = Preset {
    id: "btc-conservative",
    title: CONSERVATIVE.title,
    symbol: Some(CONSERVATIVE.symbol),
    what: "Keeps a small buy and sell order on each side of the BTC price, wide apart.",
    money: "Earns the gap when both sides fill in a calm market; loses when BTC runs one way and \
            it is left holding the losing side.",
    amounts: Amounts { min: 100, default: 100, max: 5_000 },
    needs_market: true,
    settings: |amount, market| size(&CONSERVATIVE, amount, market),
    worst_case: |amount| worst_day(CONSERVATIVE.loss_pct, amount),
};

const ETH_BALANCED: Preset = Preset {
    id: "eth-balanced",
    title: BALANCED.title,
    symbol: Some(BALANCED.symbol),
    what: "Keeps a buy and sell order on each side of the ETH price, closer in, so it trades more \
           often.",
    money: "Earns the gap on each round trip; loses more than the conservative one when ETH \
            trends, because it fills more.",
    amounts: Amounts { min: 50, default: 200, max: 5_000 },
    needs_market: true,
    settings: |amount, market| size(&BALANCED, amount, market),
    worst_case: |amount| worst_day(BALANCED.loss_pct, amount),
};

#[cfg(test)]
#[path = "describe_tests.rs"]
mod tests;
