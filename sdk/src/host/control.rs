//! The data of control commands: what a running strategy is asked to change from outside (an MCP
//! server, Telegram, the owner's UI) and what it answers. A command arrives as an
//! [`Event::Control`](crate::Event::Control) and is answered with an
//! [`Intent::Reply`](crate::Intent::Reply). Who sent it and the channel it travels on are the
//! host's.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::instance::InstanceScope;
use crate::param_spec::BoundsMode;

/// Setting values by path (the strategy's setting names, as its descriptor declares them), as text:
/// the strategy parses each and checks it against its own specs. `bounds` is the range they are
/// checked against: Guarded unless the config store authorized the change under the
/// `extended_bounds` unlock or an owner signature (never past the hard range).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingChanges {
    pub values: BTreeMap<String, String>,
    pub bounds: BoundsMode,
}

impl SettingChanges {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// These changes with `path` set to `value`.
    #[must_use]
    pub fn set(mut self, path: impl Into<String>, value: impl Into<String>) -> Self {
        self.values.insert(path.into(), value.into());
        self
    }
}

/// A live tightening of the hard limits: each value set replaces the current one, and the runtime
/// refuses the whole change if any of them would loosen a limit (a raise is a config change, which
/// rebuilds the bot). On a paper bot without hard limits, a change that completes them starts the
/// guards.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitChanges {
    pub max_position: Option<Decimal>,
    pub daily_loss_usd: Option<Decimal>,
    pub stale_secs: Option<u64>,
    pub max_notional_usd: Option<Decimal>,
    pub max_orders_per_min: Option<u32>,
}

impl LimitChanges {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A command for a running strategy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlCmd {
    /// Change settings by path (validated against the strategy's specs and journaled with the
    /// reason).
    SetSettings { changes: SettingChanges, reason: String, expected_effect: Option<String> },
    /// Tighten hard limits live (risk gate and guards); a raise is refused.
    SetLimits(LimitChanges),
    /// Cancel all quotes now and stop quoting. The position is kept.
    Pause,
    /// Clear a user pause and the reject breaker.
    Resume,
    /// Pause, then close the position with up to
    /// `CLOSE_ATTEMPTS` IOC orders at the touch, never
    /// more than `CLOSE_MAX_SLIPPAGE_BPS` from
    /// mid.
    ClosePosition,
    /// The account's kill switch (`InstanceScope::All` only): `engage` pauses every instance and
    /// cancels its orders; anyone may engage it, only the owner may release it. The supervisor's
    /// `AccountControl` answers it; a runtime alone refuses it.
    KillSwitch { scope: InstanceScope, engage: bool },
    /// Paper-trade these settings on the live market beside the running bot, in a twin of it that
    /// can never trade; staging a name again replaces that twin, and `None` is the default stage.
    /// Several named twins run side by side on the same market data. Changes nothing the bot does,
    /// so it needs no authorization; promoting the values is an ordinary `SetSettings` through the
    /// config store. A strategy that can't stage refuses it.
    Stage { changes: SettingChanges, name: Option<String>, reason: String },
    /// Drop the twin `name`, or with `None` every twin.
    Unstage { name: Option<String> },
}

/// The outcome of a command, in plain English.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlReply {
    Ok(String),
    /// The command was not carried out; the text says why.
    Refused(String),
}
