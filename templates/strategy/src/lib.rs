//! __NAME__: one post-only buy `spacing_bps` under the mid and one sell over it, `order_size`
//! each. A side that fills, is refused or is cancelled is placed again on the next book. At stop
//! it cancels everything it placed. Start here: change `quote` and `on_event`, keep the tests
//! green (`just test`), then `just build __NAME__`.
//!
//! The bot runs it behind its own safety layer: every order passes the owner's limits, paper
//! trading is the default, and the daily loss stop applies. A strategy only returns intents.

use bb_strategy_sdk::{
    Decimal, Describe, Descriptor, Event, FromSettings, Intent, Loosening, NewOrder, OrderBook,
    OrderStatus, OrderType, Range, ReportField, SettingSpec, Side, StrategyAdapter, StrategyError,
    StrategyReport, Tier, View,
};
use serde::{Deserialize, Serialize};

/// The settings, as JSON (the bot passes the owner's `[strategies.<plugin>]` section).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Distance from the mid to each order, in basis points of the mid.
    pub spacing_bps: Decimal,
    /// Size of each order.
    pub order_size: Decimal,
}

/// The strategy's state: the client id resting on each side (buy, sell), if any.
#[derive(Debug)]
pub struct __TYPE__ {
    settings: Settings,
    resting: [Option<String>; 2],
    next_id: u64,
    fills: u64,
}

fn slot(side: Side) -> usize {
    match side {
        Side::Buy => 0,
        Side::Sell => 1,
    }
}

impl __TYPE__ {
    fn next_client_id(&mut self) -> String {
        self.next_id = self.next_id.saturating_add(1);
        format!("__NAME__-{}", self.next_id)
    }

    /// The price for `side`: `spacing_bps` from the mid, on the tick when the venue gives one.
    fn price(&self, side: Side, mid: Decimal, tick: Option<Decimal>) -> Option<Decimal> {
        let offset =
            mid.checked_mul(self.settings.spacing_bps)?.checked_div(Decimal::from(10_000))?;
        let raw = match side {
            Side::Buy => mid.checked_sub(offset)?,
            Side::Sell => mid.checked_add(offset)?,
        };
        let price = match tick.filter(|t| *t > Decimal::ZERO) {
            Some(t) => {
                let steps = raw.checked_div(t)?;
                let steps = if side == Side::Buy { steps.floor() } else { steps.ceil() };
                steps.checked_mul(t)?
            }
            None => raw,
        };
        (price > Decimal::ZERO).then_some(price)
    }

    /// Place each side that has nothing resting.
    fn quote(&mut self, view: &View<'_>) -> Vec<Intent> {
        // The venue or the bot refuses new orders for now: wait.
        if view.venue_halt.is_some() || view.path_stop.is_some() {
            return Vec::new();
        }
        let Some(mid) = view.book.and_then(OrderBook::midpoint) else { return Vec::new() };
        let mut out = Vec::new();
        for side in [Side::Buy, Side::Sell] {
            if self.resting[slot(side)].is_some() {
                continue;
            }
            let Some(price) = self.price(side, mid, view.market.tick_size) else { continue };
            let client_id = self.next_client_id();
            self.resting[slot(side)] = Some(client_id.clone());
            out.push(Intent::Place(NewOrder {
                symbol: view.symbol.to_owned(),
                side,
                order_type: OrderType::PostOnly,
                price,
                quantity: self.settings.order_size,
                client_id: Some(client_id),
                reduce_only: false,
                protect: None,
            }));
        }
        out
    }

    /// Free the side whose order carries `client_id`.
    fn free(&mut self, client_id: Option<&str>) {
        for r in &mut self.resting {
            if r.is_some() && r.as_deref() == client_id {
                *r = None;
            }
        }
    }
}

impl StrategyAdapter for __TYPE__ {
    type Report = Report;

    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        Ok(self.quote(view))
    }

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent> {
        match event {
            Event::Book => self.quote(view),
            Event::Fill(trade) => {
                // This template frees a side on any fill; a real one tracks partial fills.
                self.fills = self.fills.saturating_add(1);
                self.free(trade.client_id.as_deref());
                Vec::new()
            }
            Event::Outcome { intent: Intent::Place(order), result } if !result.success => {
                self.free(order.client_id.as_deref());
                Vec::new()
            }
            Event::Order(order)
                if matches!(order.status, OrderStatus::Cancelled | OrderStatus::Rejected) =>
            {
                self.free(order.client_id.as_deref());
                Vec::new()
            }
            // An order whose outcome is unknown may rest: its side stays taken. The ladder example
            // shows how to free it after a venue read (`strategies/ladder/src/retry.rs`).
            _ => Vec::new(),
        }
    }

    fn stop(&mut self, _view: &View<'_>) -> Vec<Intent> {
        self.resting = [None, None];
        vec![Intent::CancelAll]
    }

    fn report(&self) -> Report {
        Report { resting: self.resting.iter().filter(|r| r.is_some()).count(), fills: self.fills }
    }
}

/// What the strategy shows of itself on the owner's page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub resting: usize,
    pub fills: u64,
}

impl StrategyReport for Report {
    fn headline(&self) -> String {
        format!("{} of 2 orders resting, {} fills", self.resting, self.fills)
    }

    fn fields(&self) -> Vec<ReportField> {
        vec![
            ReportField::number("resting", Decimal::from(u64::try_from(self.resting).unwrap_or(0))),
            ReportField::number("fills", Decimal::from(self.fills)),
        ]
    }
}

impl FromSettings for __TYPE__ {
    fn from_settings(settings: &str) -> Result<Self, StrategyError> {
        let settings: Settings = serde_json::from_str(settings)
            .map_err(|e| StrategyError(format!("__NAME__ settings: {e}")))?;
        if settings.spacing_bps <= Decimal::ZERO || settings.spacing_bps >= Decimal::from(10_000) {
            return Err(StrategyError(
                "__NAME__: spacing_bps must be above 0 and below 10000".into(),
            ));
        }
        if settings.order_size <= Decimal::ZERO {
            return Err(StrategyError("__NAME__: order_size must be above 0".into()));
        }
        Ok(Self { settings, resting: [None, None], next_id: 0, fills: 0 })
    }
}

impl Describe for __TYPE__ {
    /// The name the bot shows and the owner signs: the folder's name.
    const NAME: &'static str = "__NAME__";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn describe() -> Descriptor {
        Descriptor {
            summary:
                "One post-only buy under the mid and one sell over it, placed again when filled."
                    .into(),
            settings: vec![
                SettingSpec::decimal(
                    "spacing_bps",
                    Decimal::from(20),
                    Range::new(Decimal::ONE, Decimal::from(9_999)),
                    "Distance from the mid to each order, in basis points.",
                )
                .required(),
                SettingSpec::decimal(
                    "order_size",
                    Decimal::new(1, 3),
                    Range::default(),
                    "Size of each order.",
                )
                .tier(Tier::Limit)
                .loosening(Loosening::Up)
                .required(),
            ],
            settings_schema: schemars::schema_for!(Settings),
            legs: Vec::new(),
        }
    }
}

bb_strategy_sdk::export_strategy!(__TYPE__);

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
