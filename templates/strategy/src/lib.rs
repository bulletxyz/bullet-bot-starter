//! __NAME__: one post-only buy `spacing_bps` under the mid and one sell over it, `order_size`
//! each, while the position stays within `max_position` either way. When the mid moves a full
//! spacing away from a resting order, that order is cancelled and placed again at the new price.
//! A side is free again once its order is filled in full (partial fills add up), refused,
//! cancelled, or missing from a fresh venue read. At stop it cancels everything it placed. Start
//! here: change `quote` and `on_event`, keep the tests green (`just test`), then `just build
//! __NAME__`. What every type holds: `docs/SDK.md`.
//!
//! The bot runs it behind its own safety layer: every order passes the owner's limits, paper
//! trading is the default, and the daily loss stop applies. A strategy only returns intents.

use bb_strategy_sdk::{
    BULLET, CancelOrder, Decimal, Describe, Descriptor, Event, ExchangeIntent, ExchangeNeed,
    FromSettings, Intent, Loosening, MarketEvent, NewOrder, OrderBook, OrderStatus, OrderType,
    Range, ReportField, SettingSpec, Side, StrategyAdapter, StrategyError, StrategyReport, Tier,
    View,
};
use serde::{Deserialize, Serialize};

/// How long a venue read may still miss an order just sent or cancelled (ms): only a read that
/// started this long after it decides the order is gone.
pub const LIST_LAG_MS: u64 = 30_000;

/// The settings, as JSON (the bot passes the owner's settings for this plugin).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Distance from the mid to each order, in basis points of the mid.
    pub spacing_bps: Decimal,
    /// Size of each order.
    pub order_size: Decimal,
    /// The largest position it builds, long or short: a side that would pass it is not placed.
    pub max_position: Decimal,
}

/// One side's order: what was sent and how much of it has filled.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Quote {
    client_id: String,
    price: Decimal,
    filled: Decimal,
    /// When it was placed, or when its cancel was sent (the bot's clock, ms).
    sent_ms: u64,
    /// A cancel was sent and the order is not gone yet.
    cancelling: bool,
}

/// The strategy's state: the order on each side (buy, sell), if any.
#[derive(Debug)]
pub struct __TYPE__ {
    settings: Settings,
    quotes: [Option<Quote>; 2],
    /// The bot's clock at start: every client id begins with it, so ids never repeat across runs.
    started_ms: u64,
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
        format!("{}-{}", self.started_ms, self.next_id)
    }

    /// The price for `side`: `spacing_bps` from the mid, on the tick when the venue gives one.
    fn price(&self, side: Side, mid: Decimal, tick: Option<Decimal>) -> Option<Decimal> {
        let raw = match side {
            Side::Buy => mid.checked_sub(self.spacing(mid)?)?,
            Side::Sell => mid.checked_add(self.spacing(mid)?)?,
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

    /// `spacing_bps` of `mid`, in price.
    fn spacing(&self, mid: Decimal) -> Option<Decimal> {
        mid.checked_mul(self.settings.spacing_bps)?.checked_div(Decimal::from(10_000))
    }

    /// Whether one more order on `side` keeps the position within `max_position`.
    fn room(&self, side: Side, position: Decimal) -> bool {
        let after = match side {
            Side::Buy => position.checked_add(self.settings.order_size),
            Side::Sell => position.checked_sub(self.settings.order_size),
        };
        after.is_some_and(|p| p.abs() <= self.settings.max_position)
    }

    /// Place each free side, and cancel an order the mid has moved a full spacing away from (it
    /// is placed again at the new price once the venue says it is cancelled).
    fn quote(&mut self, view: &View<'_>) -> Vec<Intent> {
        // It trades one market on Bullet: the one the bot gives it.
        let (Some(bullet), Some(market)) = (view.exchange(BULLET), view.sole_market(BULLET)) else {
            return Vec::new();
        };
        // The venue or the bot refuses new orders for now: wait.
        if bullet.venue_halt.is_some() || bullet.path_stop.is_some() {
            return Vec::new();
        }
        let Some(mid) = market.book().and_then(OrderBook::midpoint) else { return Vec::new() };
        let Some(spacing) = self.spacing(mid) else { return Vec::new() };
        let mut out = Vec::new();
        for side in [Side::Buy, Side::Sell] {
            let Some(target) = self.price(side, mid, market.facts.tick_size) else { continue };
            let room = self.room(side, market.position);
            match &mut self.quotes[slot(side)] {
                Some(q)
                    if !q.cancelling
                        && q.price.checked_sub(target).is_some_and(|gap| gap.abs() >= spacing) =>
                {
                    q.cancelling = true;
                    q.sent_ms = view.now_ms;
                    // By client id: the order id stays empty.
                    out.push(Intent::on(
                        BULLET,
                        ExchangeIntent::Cancel(CancelOrder {
                            symbol: market.symbol.clone(),
                            order_id: String::new(),
                            client_id: Some(q.client_id.clone()),
                        }),
                    ));
                }
                None if room => {
                    let client_id = self.next_client_id();
                    self.quotes[slot(side)] = Some(Quote {
                        client_id: client_id.clone(),
                        price: target,
                        filled: Decimal::ZERO,
                        sent_ms: view.now_ms,
                        cancelling: false,
                    });
                    out.push(Intent::on(BULLET, ExchangeIntent::Place(NewOrder {
                        symbol: market.symbol.clone(),
                        side,
                        order_type: OrderType::PostOnly,
                        price: target,
                        quantity: self.settings.order_size,
                        client_id: Some(client_id),
                        reduce_only: false,
                        protect: None,
                    })));
                }
                // Its cancel is out, or the order rests near enough, or there is no room.
                _ => {}
            }
        }
        out
    }

    /// The slot of the order that carries `client_id`.
    fn slot_of(&self, client_id: Option<&str>) -> Option<usize> {
        let id = client_id?;
        self.quotes.iter().position(|q| q.as_ref().is_some_and(|q| q.client_id == id))
    }

    /// Free the side whose order carries `client_id`.
    fn free(&mut self, client_id: Option<&str>) {
        if let Some(at) = self.slot_of(client_id) {
            self.quotes[at] = None;
        }
    }

    /// A fresh venue read: an order it omits, sent long enough before the read began, is gone (it
    /// never landed, or its end was missed); one it lists still rests, so a cancel that did not
    /// take is sent again on the next book.
    fn reconcile(&mut self, view: &View<'_>) {
        let Some(bullet) = view.exchange(BULLET) else { return };
        let (Some(open), Some(read_ms)) = (&bullet.open_orders, bullet.read_started_ms) else { return };
        for q in &mut self.quotes {
            let Some(quote) = q.as_mut() else { continue };
            if read_ms < quote.sent_ms.saturating_add(LIST_LAG_MS) {
                continue;
            }
            if open.iter().any(|o| o.client_id.as_deref() == Some(quote.client_id.as_str())) {
                quote.cancelling = false;
            } else {
                *q = None;
            }
        }
    }

    /// Something in its market on Bullet: a book, a fill, an order update (a mark changes nothing).
    fn on_market(&mut self, event: &MarketEvent, view: &View<'_>) -> Vec<Intent> {
        match event {
            MarketEvent::Book => self.quote(view),
            MarketEvent::Fill(trade) => {
                self.fills = self.fills.saturating_add(1);
                if let Some(at) = self.slot_of(trade.client_id.as_deref())
                    && let Some(q) = self.quotes[at].as_mut()
                {
                    q.filled = q.filled.saturating_add(trade.quantity);
                    // A partial fill keeps the side taken: the rest of the order still rests.
                    if q.filled >= self.settings.order_size {
                        self.quotes[at] = None;
                    }
                }
                Vec::new()
            }
            // The venue says the order ended (a cancel is confirmed here, not by its outcome):
            // the side is free and placed again at once.
            MarketEvent::Order(order)
                if matches!(
                    order.status,
                    OrderStatus::Cancelled | OrderStatus::Rejected | OrderStatus::Filled
                ) =>
            {
                self.free(order.client_id.as_deref());
                self.quote(view)
            }
            MarketEvent::Order(_) | MarketEvent::Mark(_) => Vec::new(),
        }
    }
}

impl StrategyAdapter for __TYPE__ {
    type Report = Report;

    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        self.started_ms = view.now_ms;
        Ok(self.quote(view))
    }

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent> {
        match event {
            Event::Market { exchange, event, .. } if exchange == BULLET => self.on_market(event, view),
            Event::Tick => {
                self.reconcile(view);
                self.quote(view)
            }
            Event::Outcome { intent: Intent::On { intent: ExchangeIntent::Place(order), .. }, result }
                if !result.success =>
            {
                self.free(order.client_id.as_deref());
                Vec::new()
            }
            // An order whose outcome is unknown may rest or have filled, and a refused cancel may
            // have left it resting: its side stays taken until its events come or a fresh read
            // settles it (`reconcile`).
            _ => Vec::new(),
        }
    }

    fn stop(&mut self, view: &View<'_>) -> Vec<Intent> {
        self.quotes = [None, None];
        let symbol = view.sole_market(BULLET).map(|m| m.symbol.clone());
        vec![Intent::on(BULLET, ExchangeIntent::CancelAll(symbol))]
    }

    fn report(&self) -> Report {
        Report { resting: self.quotes.iter().filter(|q| q.is_some()).count(), fills: self.fills }
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
        if settings.max_position < settings.order_size {
            return Err(StrategyError("__NAME__: max_position must be at least order_size".into()));
        }
        Ok(Self { settings, quotes: [None, None], started_ms: 0, next_id: 0, fills: 0 })
    }
}

impl Describe for __TYPE__ {
    /// The name the bot shows and the owner signs: the folder's name.
    const NAME: &'static str = "__NAME__";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn describe() -> Descriptor {
        Descriptor {
            summary: "One post-only buy under the mid and one sell over it, moved with the mid, \
                      within a largest position."
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
                SettingSpec::decimal(
                    "max_position",
                    Decimal::new(5, 3),
                    Range::default(),
                    "The largest position it builds, long or short.",
                )
                .tier(Tier::Limit)
                .loosening(Loosening::Up)
                .required(),
            ],
            settings_schema: schemars::schema_for!(Settings),
            exchanges: vec![ExchangeNeed::new(BULLET)],
        }
    }
}

bb_strategy_sdk::export_strategy!(__TYPE__);

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
