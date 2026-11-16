//! Bullet-only funding carry: a dollar-neutral basket across many markets.
//!
//! - [`decide`]: the strategy, a pure `MarketView` to `Target`s function.
//! - [`plan`]: the targets clamped to the caps again, as IOC orders.
//! - [`actor`]: the strategy's flows ([`SimpleFundingActor`]) that read the venue each tick, place
//!   the orders, stop on a daily loss and flatten on wind-down; [`adapter`] runs them as a
//!   [`StrategyAdapter`](bb_strategy_sdk::StrategyAdapter) over the host's venue requests
//!   ([`adapter::io`]).
//! - [`config`]: `[strategies.simple-funding]`, its declared specs.
//!
//! Dollar-neutral is not price-neutral: see this crate's AGENTS.md.

pub mod actor;
pub mod adapter;
pub mod config;
pub mod cost;
pub mod decide;
pub mod limits;
pub mod plan;

pub use actor::day::{DayState, KEY_DAY_STATE};
pub use actor::report::{ReportSink, SimpleFundingReport, SimpleFundingStatus};
pub use actor::{Controls, SimpleFundingActor, VENUE};
pub use adapter::SimpleFunding;
pub use config::{SimpleFundingConfig, settings};
