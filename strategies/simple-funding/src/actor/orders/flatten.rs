//! Closing everything in the universe, retried with a wider cross each time.

use std::collections::HashMap;

use bb_strategy_sdk::{Position, Side};
use rust_decimal::Decimal;

use crate::actor::SimpleFundingActor;
use crate::adapter::io::{FlowError, Venue};
use crate::limits::{FLATTEN_ATTEMPTS, FLATTEN_SLIPPAGE_MULT};
use crate::plan::Action;

impl SimpleFundingActor {
    /// Reduce-only orders closing every universe position in `positions`.
    pub(in crate::actor) fn flatten_actions(&self, positions: &[Position]) -> Vec<Action> {
        positions
            .iter()
            .filter(|p| p.size > Decimal::ZERO && self.in_universe(&p.symbol))
            .filter_map(|p| {
                let side = match p.side? {
                    Side::Buy => Side::Sell,
                    Side::Sell => Side::Buy,
                };
                Some(Action { symbol: p.symbol.clone(), side, qty: p.size, reduce_only: true })
            })
            .collect()
    }

    pub(in crate::actor) fn fallbacks(&self, positions: &[Position]) -> HashMap<String, Decimal> {
        positions.iter().map(|p| (p.symbol.clone(), self.valuation(p))).collect()
    }

    async fn wait(&self, cx: &Venue) {
        if self.retry_wait.is_zero() {
            return;
        }
        let now = cx.now();
        cx.sleep_until(now.checked_add(self.retry_wait).unwrap_or(now)).await;
    }

    /// Close everything in the universe, retrying with a wider cross each time. `Err` when
    /// positions remain after the last attempt.
    pub(in crate::actor) async fn flatten_all(
        &mut self,
        cx: &Venue,
        why: &str,
    ) -> Result<(), FlowError> {
        let b = cx;
        for attempt in 0..=FLATTEN_ATTEMPTS {
            self.now = Some(cx.now());
            let positions = match b.get_positions().await {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(error = %e, attempt, "simple-funding: flatten could not read positions");
                    if attempt == FLATTEN_ATTEMPTS {
                        return Err(FlowError::strategy(format!(
                            "flatten ({why}): positions unreadable: {e}"
                        )));
                    }
                    self.wait(cx).await;
                    continue;
                }
            };
            let actions = self.flatten_actions(&positions);
            if actions.is_empty() {
                tracing::info!(why, "simple-funding: flat");
                return Ok(());
            }
            if attempt == FLATTEN_ATTEMPTS {
                let left: Vec<&str> = actions.iter().map(|a| a.symbol.as_str()).collect();
                tracing::error!(?left, why, "simple-funding: could not flatten");
                return Err(FlowError::strategy(format!(
                    "flatten ({why}): still open: {}",
                    left.join(", ")
                )));
            }
            let mult = Decimal::from(FLATTEN_SLIPPAGE_MULT * (attempt + 1));
            let fallback = self.fallbacks(&positions);
            tracing::warn!(why, attempt, orders = actions.len(), "simple-funding: flattening");
            self.execute(b, &actions, mult, &fallback).await;
            self.wait(cx).await;
        }
        Ok(())
    }
}
