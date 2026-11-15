//! Avellaneda-Stoikov market-making strategy.
//!
//! Quotes a bid/ask ladder around a reservation price that shifts with
//! inventory. Follows the classic finite-horizon A-S closed form (2008) as
//! implemented by Hummingbot's `avellaneda_market_making`, with standard
//! production tricks: volatility from a rolling window, min/max spread
//! clamps, fee-floor guardrail, and a hard `max_position` backstop.
//!
//! [`AvellanedaStoikov`] is a `StrategyAdapter`: it holds the model state and the ladder it
//! believes rests, and returns intents; the host owns the order path, the inventory, the clock
//! and the venue reads.

pub mod config;
pub mod model;
pub mod report;
pub mod strategy;
pub mod volatility;

pub use config::{AvellanedaStoikovConfig, settings};
pub use report::AvellanedaStoikovReport;
pub use strategy::AvellanedaStoikov;
