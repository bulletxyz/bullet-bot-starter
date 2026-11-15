//! [`SimpleFunding`]: the built-in's rules as a `StrategyAdapter` on the 0.5 world. Its flows (a tick,
//! with its venue reads and orders) are async code over the world's requests ([`crate::io`]);
//! one runs at a time, and a tick that comes while one runs is dropped (the next one reads fresh).
//!
//! The universe is the leg `universe` (venue `bullet`), whose symbols the owner sets on the bot.
//! The instance's own symbol is not traded. What this cannot do that the built-in does is listed
//! in [`crate::actor`]'s module docs and the crate's README.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

use bb_strategy_sdk::{
    Answer, Decimal, Event, Intent, LegView, StrategyAdapter, StrategyError, View, signed_position,
};

use crate::actor::{Core, UNIVERSE};
use crate::config::SimpleFundingSettings;
use crate::io::{Io, poll_once};
use crate::report::Status;

type Flow = Pin<Box<dyn Future<Output = Core> + Send>>;

enum Run {
    /// Built by `start`; no flow has run.
    Idle(Box<Core>),
    Busy(Flow),
    Gone,
}

/// The strategy.
pub struct SimpleFunding {
    settings: SimpleFundingSettings,
    io: Io,
    run: Run,
    symbols: Vec<String>,
    /// Built by `start`.
    started: bool,
}

impl std::fmt::Debug for SimpleFunding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleFunding").field("symbols", &self.symbols).finish_non_exhaustive()
    }
}

impl SimpleFunding {
    #[must_use]
    pub fn new(settings: SimpleFundingSettings) -> Self {
        Self { settings, io: Io::default(), run: Run::Gone, symbols: Vec::new(), started: false }
    }

    fn leg<'a>(view: &'a View<'_>) -> Option<&'a LegView> {
        view.legs.iter().find(|l| l.id == UNIVERSE)
    }

    /// What the host showed this call: the time, the venue's facts, a stop on new orders.
    fn absorb(&self, view: &View<'_>) {
        self.io.with(|s| {
            s.now_ms = view.now_ms;
            s.order_stop = view.path_stop.or(view.venue_halt).map(str::to_owned);
            if let Some(leg) = Self::leg(view) {
                s.facts.clone_from(&leg.market);
            }
        });
    }

    /// The universe: the leg's symbols after `only` and `exclude`, each with a positive step size.
    fn universe(&self, leg: &LegView) -> BTreeMap<String, Decimal> {
        let c = &self.settings;
        leg.symbols
            .iter()
            .filter(|s| !c.exclude.contains(s) && (c.only.is_empty() || c.only.contains(s)))
            .filter_map(|s| {
                let step = leg.market.get(s)?.step_size.filter(|v| *v > Decimal::ZERO)?;
                Some((s.clone(), step))
            })
            .collect()
    }

    fn intents(&self) -> Vec<Intent> {
        self.io
            .with(crate::io::Shared::take_requests)
            .into_iter()
            .map(|(id, leg, call)| Intent::Request { id, leg, call })
            .collect()
    }

    fn drive(&mut self) {
        if let Run::Busy(flow) = &mut self.run
            && let Poll::Ready(core) = poll_once(flow.as_mut())
        {
            self.run = Run::Idle(Box::new(core));
        }
    }

    fn begin_tick(&mut self) {
        if let Run::Idle(core) = std::mem::replace(&mut self.run, Run::Gone) {
            let mut core = *core;
            self.run = Run::Busy(Box::pin(async move {
                core.on_tick().await;
                core
            }));
        }
    }
}

impl StrategyAdapter for SimpleFunding {
    type Report = Status;

    fn start(&mut self, view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        let bad = |why: &str| Err(StrategyError(format!("simple-funding: {why}")));
        self.absorb(view);
        let Some(leg) = Self::leg(view) else {
            return bad("the leg `universe` is not configured: give it the markets to trade");
        };
        let steps = self.universe(leg);
        if steps.is_empty() {
            return bad("no tradable markets (none of the universe has a size step)");
        }
        self.io.with(|s| {
            for symbol in steps.keys() {
                let seen = s.seen.entry(symbol.clone()).or_default();
                seen.book = leg.books.get(symbol).cloned();
                seen.book_at_ms = leg.book_at_ms.get(symbol).copied();
            }
        });
        let mut core = Core::new(self.settings.clone(), self.io.clone(), &steps, view.now_ms);
        // A start over positions the venue shows is refused unless they are adopted; unknown is
        // checked by the first tick.
        if let Some(positions) = &leg.positions {
            let held = positions
                .iter()
                .filter(|p| p.size > Decimal::ZERO && steps.contains_key(&p.symbol))
                .map(|p| p.symbol.clone())
                .collect::<Vec<_>>();
            if !held.is_empty() && !self.settings.adopt_existing_positions {
                return bad(&format!(
                    "the account already holds positions in the universe that this strategy did \
                     not open ({}); close them, or set adopt_existing_positions to let it manage \
                     and flatten them",
                    held.join(", ")
                ));
            }
            core.mark_start_checked(!held.is_empty());
        }
        self.symbols = steps.into_keys().collect();
        core.publish();
        self.run = Run::Idle(Box::new(core));
        self.started = true;
        Ok(Vec::new())
    }

    fn on_event(&mut self, event: &Event, view: &View<'_>) -> Vec<Intent> {
        if !self.started {
            return Vec::new();
        }
        self.absorb(view);
        match event {
            Event::Answer { id, reply } => self.io.with(|s| s.answer(*id, reply.clone())),
            Event::Leg { leg, symbol, event } if leg == UNIVERSE => match &**event {
                Event::Mark(mark) if *mark > Decimal::ZERO => self.io.with(|s| {
                    let seen = s.seen.entry(symbol.clone()).or_default();
                    seen.mark = Some(*mark);
                    seen.mark_at_ms = Some(view.now_ms);
                }),
                Event::Book => {
                    if let Some(book) = Self::leg(view).and_then(|l| l.books.get(symbol)) {
                        self.io.with(|s| {
                            let seen = s.seen.entry(symbol.clone()).or_default();
                            seen.book = Some(book.clone());
                            seen.book_at_ms = Some(view.now_ms);
                        });
                    }
                }
                _ => {}
            },
            Event::Tick => self.begin_tick(),
            _ => {}
        }
        self.drive();
        self.intents()
    }

    fn stop(&mut self, view: &View<'_>) -> Vec<Intent> {
        self.absorb(view);
        // A flow still running is dropped: the host's flatten is the exit now.
        let refused = self.io.with(|s| s.status.refused.is_some());
        self.run = Run::Gone;
        if !self.started || refused {
            // A start it refused over positions that are not its own leaves them alone.
            return Vec::new();
        }
        let held: Vec<String> = match Self::leg(view).and_then(|l| l.positions.as_ref()) {
            Some(positions) => self
                .symbols
                .iter()
                .filter(|s| !signed_position(positions, s).is_zero())
                .cloned()
                .collect(),
            // Unknown is never flat: every market of the universe.
            None => self.symbols.clone(),
        };
        if held.is_empty() {
            return Vec::new();
        }
        vec![Intent::Leg {
            leg: UNIVERSE.into(),
            intent: Box::new(Intent::Flatten { symbols: held }),
        }]
    }

    fn report(&self) -> Status {
        self.io.with(|s| s.status.clone())
    }
}

/// The answers a request can get, for the simulator in the tests.
#[doc(hidden)]
pub type Reply = Result<Answer, bb_strategy_sdk::CallError>;
