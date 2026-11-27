//! Orders the grid let go of before it saw them gone (a pause's cancel, a level a read omitted): a read past the
//! settle margin that still lists one cancels it again, never an order the grid did not let go of.

use bb_strategy_sdk::{CancelOrder, ExchangeIntent, Intent, Order};

use crate::adapter::{GridStrategy, SETTLE_MS};

impl GridStrategy {
    /// Level `i`'s order, about to lose its level, at the bot clock `at_ms`.
    pub(in crate::adapter) fn let_go_of(&mut self, i: usize, at_ms: u64) {
        if let Some(cid) = self.state.levels[i].client_id.clone() {
            self.let_go.insert(cid, at_ms);
        }
    }

    /// A cancel for each let-go order the read that started at `read_ms` still lists, once past the
    /// settle margin (asked again only a margin later); the ones it does not list are forgotten.
    pub(in crate::adapter) fn cancel_strays(
        &mut self,
        live: &[Order],
        read_ms: u64,
    ) -> Vec<Intent> {
        let mut out = Vec::new();
        let symbol = self.config.symbol.clone();
        self.let_go.retain(|cid, at| {
            if read_ms < at.saturating_add(SETTLE_MS) {
                return true;
            }
            let Some(order) = live.iter().find(|o| o.client_id.as_deref() == Some(cid.as_str()))
            else {
                return false;
            };
            tracing::warn!(client_id = %cid, "A grid order let go of still rests: cancelling it");
            out.push(Self::on_bullet(ExchangeIntent::Cancel(CancelOrder {
                symbol: symbol.clone(),
                order_id: order.id.clone(),
                client_id: Some(cid.clone()),
            })));
            *at = read_ms;
            true
        });
        out
    }
}
