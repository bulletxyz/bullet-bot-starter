use super::*;
use crate::history::candles::{Candle, CandleInterval, HistoryNeed, Warmup};

/// What the warmer was handed (its own static: the echo test shares `SEEN`).
static WARMED: Mutex<Vec<Warmup>> = Mutex::new(Vec::new());

/// Asks for 15m bars and keeps the warm-up it is handed.
#[derive(Debug)]
struct Warmer;

impl FromSettings for Warmer {
    fn from_settings(_settings: &str) -> Result<Self, StrategyError> {
        Ok(Warmer)
    }
}

impl StrategyAdapter for Warmer {
    type Report = EchoReport;

    fn start(&mut self, _view: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        Ok(Vec::new())
    }

    fn on_event(&mut self, _event: &Event, _view: &View<'_>) -> Vec<Intent> {
        Vec::new()
    }

    fn stop(&mut self, _view: &View<'_>) -> Vec<Intent> {
        Vec::new()
    }

    fn report(&self) -> EchoReport {
        EchoReport { ticks: 0 }
    }

    fn history(&self) -> Option<HistoryNeed> {
        Some(HistoryNeed { interval: CandleInterval::M15, want: 50, need: 20 })
    }

    fn warm_up(&mut self, warmup: Warmup) {
        WARMED.lock().unwrap().push(warmup);
    }
}

/// The need crosses as the strategy said it, and each warm-up comes back as the host sent it; bars
/// the host could not have sent arrive as none, said plainly.
#[test]
fn the_history_need_and_the_warm_up_cross_the_wire() {
    assert!(history().is_none(), "nothing asks before init");
    warm_up(wire::history::warmup_to_wire(&Warmup::Unavailable("early".into())));
    init::<Warmer>("{}").unwrap();
    let need = history().unwrap();
    assert!(matches!(need.interval, w::CandleInterval::M15));
    assert_eq!((need.want, need.need), (50, 20));

    let bar = Candle {
        open_ms: 1_794_397_500_000,
        open: d("75.5"),
        high: d("76.25"),
        low: d("-0.000001"),
        close: d("75.75"),
        volume: d("12.5"),
    };
    let loaded =
        Warmup::Loaded { candles: vec![bar], source: "Binance".into(), now_ms: 1_794_398_400_123 };
    let none = Warmup::Unavailable("no source of past prices is set up".into());
    warm_up(wire::history::warmup_to_wire(&loaded));
    warm_up(wire::history::warmup_to_wire(&none));
    let mut bad = wire::history::warmup_to_wire(&loaded);
    if let w::PastCandles::Loaded(l) = &mut bad {
        l.candles[0].close.scale = 29;
    }
    warm_up(bad);
    let warmed = WARMED.lock().unwrap().clone();
    assert_eq!(warmed[..2], [loaded, none], "the early one is dropped");
    assert!(
        matches!(&warmed[2], Warmup::Unavailable(why) if why.contains("did not decode")),
        "{:?}",
        warmed[2]
    );
}
