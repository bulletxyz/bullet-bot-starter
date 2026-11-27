//! The ladder's bookkeeping: the slots the strategy believes rest on the venue, and the intents of
//! a round still waiting for their answer.

use bb_strategy_sdk::{AmendOrder, BULLET, CancelOrder, ExchangeIntent, Intent, NewOrder, Side};
use rust_decimal::Decimal;

/// One rung of the ladder as the strategy tracks it. `placed_at_ms` is the bot clock when the
/// order was acknowledged.
#[derive(Debug, Clone)]
pub(crate) struct QuoteSlot {
    pub side: Side,
    pub level: usize,
    pub price: Decimal,
    pub size: Decimal,
    pub client_id: Option<String>,
    pub order_id: Option<String>,
    pub placed_at_ms: Option<u64>,
}

/// What a round asked of the host for a slot, held until the answer says what became of it.
#[derive(Debug, Clone)]
pub(crate) enum InFlight {
    /// The slot's order is being cancelled: a failed cancel keeps tracking it.
    Cancel(QuoteSlot),
    /// `old` is being replaced by `new`: a failed amend keeps `old`.
    Amend { old: QuoteSlot, new: QuoteSlot },
    /// `new` is being placed: a failed place leaves no slot.
    Place(QuoteSlot),
}

impl InFlight {
    /// The key its intent and its answer share: the client id of the order it sends or cancels.
    pub fn key(&self) -> String {
        match self {
            Self::Cancel(slot) => {
                cancel_key(slot.order_id.as_deref().unwrap_or_default(), slot.client_id.as_deref())
            }
            Self::Amend { new, .. } | Self::Place(new) => new.client_id.clone().unwrap_or_default(),
        }
    }
}

fn cancel_key(order_id: &str, client_id: Option<&str>) -> String {
    format!("{order_id}/{}", client_id.unwrap_or_default())
}

/// The key of the answer to `intent`, when it is one of the ladder's own.
pub(crate) fn key_of(intent: &Intent) -> Option<String> {
    let Intent::On { exchange, intent } = intent else { return None };
    if exchange != BULLET {
        return None;
    }
    match intent {
        ExchangeIntent::Place(o) => Some(o.client_id.clone().unwrap_or_default()),
        ExchangeIntent::Amend(AmendOrder { new_order, .. }) => {
            Some(new_order.client_id.clone().unwrap_or_default())
        }
        ExchangeIntent::Cancel(CancelOrder { order_id, client_id, .. }) => {
            Some(cancel_key(order_id, client_id.as_deref()))
        }
        _ => None,
    }
}

/// What one pass over the ladder decided, before any of it is sent.
#[derive(Default)]
pub(crate) struct Plan {
    /// Slots left as they are.
    pub kept: Vec<QuoteSlot>,
    pub cancels: Vec<(CancelOrder, QuoteSlot)>,
    pub amends: Vec<(AmendOrder, QuoteSlot, QuoteSlot)>,
    pub places: Vec<(NewOrder, QuoteSlot)>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.cancels.is_empty() && self.amends.is_empty() && self.places.is_empty()
    }
}
