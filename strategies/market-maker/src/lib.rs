//! The bot's market maker (`simple-mm`): one bid and one ask around a fair value (the Binance mid
//! plus the basis to Bullet, or the Bullet mid), with optional inventory skew and volatility
//! widening, capped at the position and notional limits. A pure strategy on the SDK port: the
//! host runs its orders, paper, guards, pause, limits and state.

pub mod adapter;
mod cap;
mod describe;
pub mod params;
pub mod pricing;
pub mod report;
pub mod settings;

pub use adapter::MarketMaker;
pub use params::{FairValue, MmParams};
pub use report::MarketMakerReport;
pub use settings::MarketMakerSettings;

#[cfg(test)]
mod purity_tests;
