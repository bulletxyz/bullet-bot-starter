//! The funding arb blueprint: [`strategy::decide`] (entry, exit, sizing, symbol choice) behind
//! [`StrategyAdapter`](bb_strategy_sdk::StrategyAdapter): a market view in, `Intent::Targets` out.
//! bb-funding's executor carries the targets out.

pub mod strategy;

pub use strategy::{FundingArb, FundingArbReport, decide};
