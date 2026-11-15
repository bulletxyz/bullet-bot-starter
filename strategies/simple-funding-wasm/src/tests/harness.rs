//! A simulated Bullet behind the `universe` leg: it answers the strategy's requests the way the
//! host would (reads from its state, IOC places that fill at once), and feeds the answers back.

#![expect(clippy::arithmetic_side_effects, reason = "test arithmetic")]

use std::collections::{BTreeMap, BTreeSet};

use bb_strategy_sdk::testing::{Market, book, d};
use bb_strategy_sdk::{
    Answer, CallError, Decimal, Event, FromSettings, FundingSample, Intent, LegView, MarketFacts,
    NewOrder, OrderResult, Position, ReadQuery, Side, StrategyAdapter, VenueCall,
};

use crate::SimpleFunding;

pub const T0: u64 = 20_000 * 86_400_000;
pub const HOUR: u64 = 3_600_000;

/// What the simulated venue holds and refuses.
#[derive(Default)]
pub struct Venue {
    /// Signed base per symbol.
    pub held: BTreeMap<String, Decimal>,
    pub equity: Option<Decimal>,
    pub margin: Option<Decimal>,
    pub positions_down: bool,
    /// Symbols whose orders the venue refuses.
    pub reject: BTreeSet<String>,
    /// Symbols whose orders error after they may have landed (they do land).
    pub ambiguous: BTreeSet<String>,
    /// A symbol whose opening orders fill only this share.
    pub partial: Option<(String, Decimal)>,
    pub funding: BTreeMap<String, Decimal>,
    pub oi: Decimal,
    pub placed: Vec<NewOrder>,
}

impl Venue {
    pub fn position_list(&self) -> Vec<Position> {
        self.held
            .iter()
            .filter(|(_, v)| !v.is_zero())
            .map(|(symbol, v)| Position {
                symbol: symbol.clone(),
                side: Some(if *v > Decimal::ZERO { Side::Buy } else { Side::Sell }),
                size: v.abs(),
                entry_price: d("100"),
                unrealized_pnl: Decimal::ZERO,
            })
            .collect()
    }

    fn history(&self, symbol: &str, now_ms: u64) -> Vec<FundingSample> {
        let rate = self.funding.get(symbol).copied().unwrap_or(Decimal::ZERO);
        let newest = i64::try_from(now_ms / HOUR).unwrap();
        (0..26)
            .map(|k| FundingSample { time_ms: (newest - k) * 3_600_000 + 1, rate_8h: rate })
            .collect()
    }

    fn place(&mut self, order: &NewOrder) -> Result<Answer, CallError> {
        self.placed.push(order.clone());
        let ok = |success: bool, error: Option<&str>| {
            Ok(Answer::Placed(vec![OrderResult {
                order_id: success.then(|| "o".to_owned()),
                client_id: order.client_id.clone(),
                success,
                error: error.map(str::to_owned),
            }]))
        };
        if self.reject.contains(&order.symbol) {
            return ok(false, Some("rejected"));
        }
        let mut qty = order.quantity;
        if let Some((symbol, share)) = &self.partial
            && !order.reduce_only
            && *symbol == order.symbol
        {
            qty = (qty * *share * d("10")).floor() / d("10");
        }
        let signed = if order.side == Side::Buy { qty } else { -qty };
        let held = self.held.entry(order.symbol.clone()).or_default();
        // A reduce-only order never grows or flips a position.
        let next = *held + signed;
        *held = if order.reduce_only && (next.is_sign_negative() != held.is_sign_negative()) {
            Decimal::ZERO
        } else {
            next
        };
        if self.ambiguous.contains(&order.symbol) {
            return Err(CallError {
                message: "timeout".into(),
                never_sent: false,
                retryable: true,
            });
        }
        ok(true, None)
    }

    fn answer(
        &mut self,
        call: &VenueCall,
        now_ms: u64,
        books: &BTreeMap<String, bb_strategy_sdk::OrderBook>,
    ) -> Result<Answer, CallError> {
        let down = || CallError { message: "down".into(), never_sent: false, retryable: true };
        match call {
            VenueCall::Read(ReadQuery::Positions) if self.positions_down => Err(down()),
            VenueCall::Read(ReadQuery::Positions) => Ok(Answer::Positions(self.position_list())),
            VenueCall::Read(ReadQuery::AccountEquity) => {
                Ok(Answer::AccountEquity(self.equity.map(|e| bb_strategy_sdk::AccountEquity {
                    equity: e,
                    wallet_balance: e,
                    unrealized_pnl: Decimal::ZERO,
                })))
            }
            VenueCall::Read(ReadQuery::MarginRatio) => Ok(Answer::MarginRatio(self.margin)),
            VenueCall::Read(ReadQuery::Book { symbol, .. }) => {
                books.get(symbol).cloned().map(Answer::Book).ok_or_else(down)
            }
            VenueCall::Read(ReadQuery::FundingHistory { symbol, .. }) => {
                Ok(Answer::FundingHistory(self.history(symbol, now_ms)))
            }
            VenueCall::Read(ReadQuery::OpenInterest { .. }) => {
                Ok(Answer::OpenInterest(Some(self.oi)))
            }
            VenueCall::Read(ReadQuery::RefreshMetadata) => Ok(Answer::Done),
            VenueCall::Place(orders) => self.place(&orders[0]),
            other => panic!("unexpected call {other:?}"),
        }
    }
}

/// The strategy, its venue and the host's view of the `universe` leg.
pub struct Sim {
    pub strategy: SimpleFunding,
    pub venue: Venue,
    pub market: Market,
    pub books: BTreeMap<String, bb_strategy_sdk::OrderBook>,
    /// Every request the strategy made, in order.
    pub requests: Vec<VenueCall>,
}

pub fn facts() -> MarketFacts {
    MarketFacts {
        step_size: Some(d("0.1")),
        taker_fee_bps: Some(Decimal::ZERO),
        ..MarketFacts::default()
    }
}

pub fn settings(extra: &str) -> SimpleFunding {
    SimpleFunding::from_settings(&format!("{{{extra}}}")).unwrap()
}

impl Sim {
    /// `symbols` on the leg, each with a deep 99.95 / 100.05 book, a step of 0.1, fee 0; a symbol
    /// starting with `L` pays longs, any other pays shorts.
    pub fn new(strategy: SimpleFunding, symbols: &[&str]) -> Self {
        let mut venue = Venue {
            equity: Some(d("10000")),
            margin: Some(d("0.1")),
            oi: d("1000000"),
            ..Venue::default()
        };
        let mut leg =
            LegView::new("universe", "bullet", symbols.iter().map(|s| (*s).to_owned()).collect());
        let mut books = BTreeMap::new();
        for s in symbols {
            venue.funding.insert(
                (*s).to_owned(),
                if s.starts_with('L') { d("-0.0006") } else { d("0.0006") },
            );
            leg.market.insert((*s).to_owned(), facts());
            let b = book("99.95", "100.05", "1000");
            leg.books.insert((*s).to_owned(), b.clone());
            leg.book_at_ms.insert((*s).to_owned(), T0);
            books.insert((*s).to_owned(), b);
        }
        leg.positions = Some(Vec::new());
        let mut market = Market::new("BTC-USD", "99.95", "100.05", T0);
        market.legs = vec![leg];
        Self { strategy, venue, market, books, requests: Vec::new() }
    }

    pub fn start(&mut self) -> Result<Vec<Intent>, bb_strategy_sdk::StrategyError> {
        self.strategy.start(&self.market.view())
    }

    /// Send `event`, answer every request until the strategy asks for no more; every intent.
    pub fn send(&mut self, event: Event) -> Vec<Intent> {
        let mut all = Vec::new();
        let mut next = vec![event];
        while let Some(ev) = next.pop() {
            for intent in self.strategy.on_event(&ev, &self.market.view()) {
                if let Intent::Request { id, call, .. } = &intent {
                    self.requests.push(call.clone());
                    let reply = self.venue.answer(call, self.market.now_ms, &self.books);
                    // The host's view carries the leg's positions as this read found them.
                    if let Some(leg) = self.market.legs.first_mut() {
                        leg.positions = Some(self.venue.position_list());
                    }
                    next.push(Event::Answer { id: *id, reply });
                }
                all.push(intent);
            }
        }
        all
    }

    /// Marks and books for every symbol at the current time, then a tick.
    pub fn tick(&mut self) -> Vec<Intent> {
        let symbols = self.market.legs[0].symbols.clone();
        for s in &symbols {
            self.market.legs[0].book_at_ms.insert(s.clone(), self.market.now_ms);
            self.send(Event::Leg {
                leg: "universe".into(),
                symbol: s.clone(),
                event: Box::new(Event::Mark(d("100"))),
            });
            self.send(Event::Leg {
                leg: "universe".into(),
                symbol: s.clone(),
                event: Box::new(Event::Book),
            });
        }
        self.send(Event::Tick)
    }

    pub fn advance(&mut self, secs: u64) {
        self.market.now_ms += secs * 1_000;
    }

    /// The orders the strategy placed through the leg, in order.
    pub fn orders(&self) -> &[NewOrder] {
        &self.venue.placed
    }

    pub fn held(&self, symbol: &str) -> Decimal {
        self.venue.held.get(symbol).copied().unwrap_or(Decimal::ZERO)
    }
}
