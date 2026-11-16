//! simple-funding as a plain [`StrategyAdapter`]: the strategy's flows (a tick's reads and orders,
//! a start, a flatten) run on a poller with no runtime ([`io`]). Each venue call a flow
//! makes leaves as an [`Intent::Request`] the host carries out through the leg's order path, and
//! the flow resumes on its [`Event::Answer`]; its alerts and its day state leave as
//! [`Intent::Notify`] and [`Intent::Persist`]. One flow runs at a time: an event that comes
//! meanwhile waits its turn (ticks, marks and books of a symbol are coalesced to the latest).

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use bb_strategy_sdk::{Event, Intent, OrderBook, StrategyAdapter, StrategyError, View};
use rust_decimal::Decimal;

use crate::VENUE;
use crate::actor::SimpleFundingActor;
use crate::actor::report::SimpleFundingStatus;
use crate::adapter::io::{FlowError, Shared, Venue};

type Flow = Pin<Box<dyn Future<Output = (SimpleFundingActor, Result<(), FlowError>)> + Send>>;

/// What a flow is: what its error means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Start,
    Event,
    Stop,
}

/// What the flows are shown: the leg's marks and books and the bot's ticks.
enum Ev {
    Tick,
    Mark { symbol: String, price: Decimal, funding: Option<Decimal> },
    Book { symbol: String, book: Arc<OrderBook> },
}

/// The strategy behind the host.
pub struct SimpleFunding {
    idle: Option<SimpleFundingActor>,
    flow: Option<(Kind, Flow)>,
    queue: VecDeque<Ev>,
    shared: Shared,
    report: SimpleFundingStatus,
}

impl SimpleFunding {
    #[must_use]
    pub fn new(strategy: SimpleFundingActor) -> Self {
        let shared = strategy.shared();
        let report = SimpleFundingStatus { status: strategy.status() };
        Self { idle: Some(strategy), flow: None, queue: VecDeque::new(), shared, report }
    }

    /// What the host shows now: the bot's time and the leg's facts.
    fn see(&self, view: &View<'_>) {
        let mut calls = self.shared.lock();
        calls.now_ms = view.now_ms;
        if let Some(leg) = view.legs.iter().find(|l| l.venue == VENUE) {
            calls.leg.clone_from(&leg.id);
            calls.symbols.clone_from(&leg.symbols);
            calls.market.clone_from(&leg.market);
        }
    }

    /// `event` as the flows see it, queued (a newer tick, mark or book replaces the one waiting).
    fn queue(&mut self, event: Ev) {
        let same = |e: &Ev| match (e, &event) {
            (Ev::Tick, Ev::Tick) => true,
            (Ev::Mark { symbol: a, .. }, Ev::Mark { symbol: b, .. })
            | (Ev::Book { symbol: a, .. }, Ev::Book { symbol: b, .. }) => a == b,
            _ => false,
        };
        if let Some(slot) = self.queue.iter_mut().find(|e| same(e)) {
            *slot = event;
        } else {
            self.queue.push_back(event);
        }
    }

    fn begin(&mut self, kind: Kind, event: Option<Ev>) {
        let Some(mut strategy) = self.idle.take() else { return };
        let venue = Venue::new(self.shared.clone());
        let flow: Flow = Box::pin(async move {
            let done = match (kind, event) {
                (Kind::Start, _) => strategy.start(&venue).await,
                (Kind::Stop, _) => strategy.stop(&venue).await,
                (Kind::Event, Some(Ev::Tick)) => strategy.tick(&venue).await,
                (Kind::Event, Some(Ev::Mark { symbol, price, funding })) => {
                    strategy.on_mark(&symbol, price, funding, &venue);
                    Ok(())
                }
                (Kind::Event, Some(Ev::Book { symbol, book })) => {
                    strategy.on_book(&symbol, book, &venue);
                    Ok(())
                }
                (Kind::Event, None) => Ok(()),
            };
            (strategy, done)
        });
        self.flow = Some((kind, flow));
    }

    /// Run the flows as far as the answers in hand let them: what they ask is the host's to do.
    fn run(&mut self) -> Vec<Intent> {
        let mut out = Vec::new();
        loop {
            if self.flow.is_none() {
                match self.queue.pop_front() {
                    Some(event) if self.idle.is_some() => self.begin(Kind::Event, Some(event)),
                    Some(event) => {
                        self.queue.push_front(event);
                        break;
                    }
                    None => break,
                }
            }
            let Some((kind, flow)) = self.flow.as_mut() else { break };
            let mut task = Context::from_waker(Waker::noop());
            let Poll::Ready((strategy, done)) = flow.as_mut().poll(&mut task) else {
                // Waiting with nothing out: nothing the host does will ever wake it.
                if !self.shared.waits_on_host() {
                    let why = "strategy: a flow waits on something the host does not serve";
                    tracing::error!("{why}");
                    out.push(Intent::Shutdown(why.into()));
                }
                break;
            };
            let kind = *kind;
            self.flow = None;
            self.report = SimpleFundingStatus { status: strategy.status() };
            self.idle = Some(strategy);
            if let Err(e) = done {
                match kind {
                    Kind::Start => out.push(Intent::Shutdown(format!("start refused: {e}"))),
                    Kind::Stop => out.push(Intent::Shutdown(format!("stop failed: {e}"))),
                    Kind::Event if e.is_fatal() => out.push(Intent::Shutdown(e.to_string())),
                    Kind::Event => tracing::warn!(error = %e, "strategy: an event's flow failed"),
                }
            }
            if kind == Kind::Stop {
                break;
            }
        }
        let mut calls = self.shared.lock();
        out.extend(calls.out.drain(..));
        if let Some(why) = calls.shutdown.take() {
            out.push(Intent::Shutdown(why));
        }
        out
    }

    /// `event` from the host as the flows see it, if it is one.
    fn ev(event: &Event, view: &View<'_>) -> Option<Ev> {
        let Event::Leg { leg, symbol, event } = event else {
            return matches!(event, Event::Tick).then_some(Ev::Tick);
        };
        let at = view.legs.iter().find(|l| &l.id == leg)?;
        match &**event {
            Event::Mark(price) => Some(Ev::Mark {
                symbol: symbol.clone(),
                price: *price,
                funding: at.funding_rates.get(symbol).copied(),
            }),
            Event::Book => {
                Some(Ev::Book { symbol: symbol.clone(), book: Arc::clone(at.books.get(symbol)?) })
            }
            _ => None,
        }
    }
}

impl StrategyAdapter for SimpleFunding {
    type Report = SimpleFundingStatus;

    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        self.see(view);
        self.begin(Kind::Start, None);
        let out = self.run();
        match out.iter().find_map(|i| match i {
            Intent::Shutdown(why) if self.flow.is_none() => Some(why.clone()),
            _ => None,
        }) {
            Some(why) => Err(StrategyError(why)),
            None => Ok(out),
        }
    }

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent> {
        self.see(view);
        match event {
            Event::Answer { id, reply } => {
                self.shared.answer(*id, Some(reply.clone().map_err(io::to_bot)));
            }
            Event::Persisted { id, result } => self.shared.persisted(*id, result.clone()),
            other => {
                if let Some(event) = Self::ev(other, view) {
                    self.queue(event);
                }
            }
        }
        self.run()
    }

    fn stop(&mut self, view: &View<'_>) -> Vec<Intent> {
        self.see(view);
        if self.flow.is_some() {
            tracing::error!("strategy: a flow is still running at its stop; the stop cannot run");
            return vec![Intent::Shutdown("stop failed: a flow did not end".into())];
        }
        self.queue.clear();
        self.begin(Kind::Stop, None);
        self.run()
    }

    fn report(&self) -> SimpleFundingStatus {
        self.idle
            .as_ref()
            .map_or_else(|| self.report.clone(), |s| SimpleFundingStatus { status: s.status() })
    }
}

pub mod io;
