//! The guest SDK for bullet-bots strategies. A strategy implements [`StrategyAdapter`] (data in,
//! intents out), [`FromSettings`] and [`Describe`], and `export_strategy!(MyStrategy)` makes the
//! crate a `bb:strategy@0.5.0` component when it is built for `wasm32-wasip2` (`crate-type =
//! ["cdylib", "rlib"]`). The same code runs natively in its own tests.
//!
//! Pure: no tokio, no I/O, no clock. The port's shape is bb-strategy's (`StrategyAdapter`,
//! `StrategyReport`, `View`, `Event`, `Intent`); the host converts between the two at the boundary.

extern crate self as bb_strategy_sdk;

mod adapter;
mod calls;
pub mod decimal;
mod describe;
#[doc(hidden)]
pub mod guest;
mod history;
mod legs;
#[cfg(feature = "testing")]
pub mod testing;
mod types;
mod wire;
mod wire_calls;
mod wire_history;

pub use adapter::{
    Event, FieldValue, FromSettings, Intent, MarketFacts, PaperInputs, ReportField,
    StrategyAdapter, StrategyError, StrategyReport, View,
};
pub use calls::{Answer, CallError, ReadQuery, VenueCall};
pub use describe::{
    Describe, Descriptor, LegNeed, Loosening, Range, SettingKind, SettingSpec, Tier,
};
pub use history::{Candle, CandleInterval, HistoryNeed, MAX_HISTORY_CANDLES, Warmup};
pub use legs::{LegView, TRADE_LEG};
pub use rust_decimal::{Decimal, RoundingStrategy};
pub use types::{
    AccountEquity, AmendOrder, CancelOrder, CancelResult, FundingSample, Level, NewOrder, Order,
    OrderBook, OrderResult, OrderStatus, OrderType, Position, Protection, Side, Trade,
    signed_position,
};

/// The version of the `bb:strategy` world this SDK builds components for.
pub const WORLD_VERSION: &str = "0.5.0";

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
