//! RSI momentum on one Bullet market.
//!
//! - [`indicator::candles`]: OHLC candles from the mark feed, closed on interval boundaries by
//!   event time.
//! - [`indicator::rsi`]: Wilder's RSI over the closes.
//! - [`decide`]: the pure rules: crossing entries, band exits, the stop, the entry size.
//! - [`adapter`]: [`RsiMomentum`], the strategy behind the host (`StrategyAdapter`): candles,
//!   signals and the IOCs, closes and stop it asks for as intents; the host owns the position,
//!   limits, paper, daily loss stop and flatten.
//! - [`report`]: what it shows of itself in the host's snapshot.
//! - [`config`]: `[strategies.rsi-momentum]`, its declared settings.

pub mod adapter;
pub mod config;
pub mod decide;
mod describe;
pub mod indicator;
pub mod limits;
pub mod report;

pub use adapter::RsiMomentum;
pub use config::{RsiMomentumConfig, settings};
pub use report::RsiReport;
