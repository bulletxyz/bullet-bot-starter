//! The guest SDK for bullet-bots strategies. A strategy implements [`StrategyAdapter`] (data in,
//! intents out), [`FromSettings`] and [`Describe`], and `export_strategy!(MyStrategy)` makes the
//! crate a `bb:strategy@0.6.0` component when it is built for `wasm32-wasip2` (`crate-type =
//! ["cdylib", "rlib"]`). The same code runs natively in its own tests.
//!
//! Pure: no tokio, no I/O, no clock. This is the one port: the bot's own built-in strategies run on
//! the same [`StrategyAdapter`], natively, and the bot re-exports these types as its own, so a
//! strategy is the same code in both. A plugin (a component) is sent and may return only what the
//! `bb:strategy` world carries; the rest (`Reference`, `Control`, `Persist`, `Targets`, ...) is the
//! bot's own, and a component that returns one faults.

extern crate self as bb_strategy_sdk;

mod component;
mod config;
mod guards;
mod history;
mod host;
pub mod instance;
mod strategy;
#[cfg(feature = "testing")]
pub mod testing;
mod types;

pub use component::decimal;
#[doc(hidden)]
pub use component::guest;
pub use config::{param_spec, text_format};
pub use guards::{alert_keys, stops};
pub use host::targets;

pub use config::setting::{HostLimit, Setting, ValidateConfig};
pub use erased_serde::Serialize as ErasedSerialize;
pub use history::candles::{Candle, CandleInterval, HistoryNeed, MAX_HISTORY_CANDLES, Warmup};
pub use history::records::{
    Fees, FillKind, FillRecord, HistoryEvent, HistoryReads, Restored, WindowMetrics,
};
pub use host::calls::{Answer, CallError, Cause, ReadQuery, VenueCall};
pub use host::capability::{Alert, CloseReport, LimitsUpdate, Persist, ReferencePrice};
pub use host::control::{ControlCmd, ControlReply, LimitChanges, SettingChanges};
pub use host::exchanges::{BULLET, ExchangeView, MarketView};
pub use rust_decimal::{Decimal, RoundingStrategy};
pub use strategy::adapter::{
    Event, ExchangeIntent, FieldValue, FromSettings, Intent, LimitsInForce, MarketEvent,
    MarketFacts, OwnBook, PaperInputs, ReportField, SnapshotExtras, StartMode, StateRead,
    StrategyAdapter, StrategyError, StrategyReport, View,
};
pub use strategy::describe::{
    DEFAULT_RESYNC_TICKS, Describe, Descriptor, ExchangeNeed, Feed, Loosening, Needs, Range,
    SettingKind, SettingSpec, Tier,
};
pub use strategy::erased::ErasedAdapter;
pub use strategy::profile;
pub use strategy::profile::{About, ExchangeRole, Featured, Preset, Profile, Risk, RiskLevel};
pub use strategy::report::{
    MAX_FIELD_NAME, MAX_FIELD_TEXT, MAX_FIELD_UNIT, MAX_HEADLINE, MAX_REPORT_FIELDS, fields_json,
    fields_of,
};
pub use strategy::twin::{NoHistory, TwinAdapter, TwinHistory, TwinSpec};
pub use types::{
    AccountEquity, AmendOrder, Balance, CancelOrder, CancelResult, FundingSample, Level, NewOrder,
    Order, OrderBook, OrderResult, OrderStatus, OrderType, Position, Protection, ReplaceResult,
    Side, StopKind, Trade, VenueStop, signed_position,
};

/// The version of the `bb:strategy` world this SDK builds components for.
pub const WORLD_VERSION: &str = "0.6.0";

/// The world's generated bindings (wit-bindgen over `wit/strategy.wit`).
#[doc(hidden)]
#[expect(clippy::same_length_and_capacity, reason = "wit-bindgen's generated list lifting")]
pub mod __bindings {
    wit_bindgen::generate!({
        world: "strategy",
        path: "wit",
        pub_export_macro: true,
        export_macro_name: "__export_strategy_world",
        default_bindings_module: "::bb_strategy_sdk::__bindings",
        runtime_path: "::bb_strategy_sdk::__rt",
    });
}

#[doc(hidden)]
pub use wit_bindgen::rt as __rt;

/// Export `$strategy` (a [`StrategyAdapter`] + [`FromSettings`] + [`Describe`]) as the component's
/// `bb:strategy/adapter` interface. Once per component crate, at its root.
#[macro_export]
macro_rules! export_strategy {
    ($strategy:ty) => {
        #[doc(hidden)]
        pub struct __BbStrategyComponent;

        // The name and version, readable from the file without running it: the host reads this
        // section before the owner approves the file.
        #[cfg(target_arch = "wasm32")]
        #[used]
        #[allow(unsafe_code, reason = "a link_section attribute, the only way to write a custom section")]
        #[unsafe(link_section = "bb-strategy-meta")]
        static __BB_STRATEGY_META: [u8; <$strategy as $crate::Describe>::NAME.len()
            + 1
            + <$strategy as $crate::Describe>::VERSION.len()] = $crate::guest::meta(
            <$strategy as $crate::Describe>::NAME,
            <$strategy as $crate::Describe>::VERSION,
        );

        impl $crate::guest::w::Guest for __BbStrategyComponent {
            fn describe() -> $crate::guest::w::Descriptor {
                $crate::guest::describe::<$strategy>()
            }

            fn init(settings: String) -> Result<(), String> {
                $crate::guest::init::<$strategy>(&settings)
            }

            fn history() -> Option<$crate::guest::w::HistoryNeed> {
                $crate::guest::history()
            }

            fn warm_up(warmup: $crate::guest::w::PastCandles) {
                $crate::guest::warm_up(warmup);
            }

            fn start(
                view: $crate::guest::w::View,
            ) -> Result<Vec<$crate::guest::w::Intent>, String> {
                $crate::guest::start(&view)
            }

            fn on_event(
                event: $crate::guest::w::Event,
                view: $crate::guest::w::View,
            ) -> Vec<$crate::guest::w::Intent> {
                $crate::guest::on_event(event, &view)
            }

            fn stop(view: $crate::guest::w::View) -> Vec<$crate::guest::w::Intent> {
                $crate::guest::stop(&view)
            }

            fn report() -> $crate::guest::w::ReportRecord {
                $crate::guest::report()
            }
        }

        // Only a component exports the world: a native build (tests, doc tests) would link the
        // `#`-named export symbols, which a native linker refuses.
        #[cfg(target_arch = "wasm32")]
        $crate::__bindings::__export_strategy_world!(__BbStrategyComponent with_types_in $crate::__bindings);
    };
}
