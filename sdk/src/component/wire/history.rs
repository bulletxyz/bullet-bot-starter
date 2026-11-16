//! Past candles at start ([`HistoryNeed`], [`Warmup`]) to and from the world's types.

use super::{undec, w};
use crate::history::candles::{Candle, CandleInterval, HistoryNeed, Warmup};

fn interval_to_wire(value: CandleInterval) -> w::CandleInterval {
    match value {
        CandleInterval::M1 => w::CandleInterval::M1,
        CandleInterval::M5 => w::CandleInterval::M5,
        CandleInterval::M15 => w::CandleInterval::M15,
        CandleInterval::M30 => w::CandleInterval::M30,
        CandleInterval::H1 => w::CandleInterval::H1,
        CandleInterval::H4 => w::CandleInterval::H4,
        CandleInterval::D1 => w::CandleInterval::D1,
    }
}

pub(crate) fn history_need_to_wire(value: HistoryNeed) -> w::HistoryNeed {
    w::HistoryNeed {
        interval: interval_to_wire(value.interval),
        want: value.want,
        need: value.need,
    }
}

fn candle(c: &w::Candle) -> Result<Candle, crate::decimal::BadDecimal> {
    Ok(Candle {
        open_ms: c.open_ms,
        open: undec(c.open)?,
        high: undec(c.high)?,
        low: undec(c.low)?,
        close: undec(c.close)?,
        volume: undec(c.volume)?,
    })
}

/// What the host loaded, as the strategy takes it. The host encodes only valid decimals; bars it
/// could not have sent are no bars, said plainly.
pub(crate) fn warmup_from_wire(value: w::PastCandles) -> Warmup {
    match value {
        w::PastCandles::Loaded(l) => match l.candles.iter().map(candle).collect() {
            Ok(candles) => Warmup::Loaded { candles, source: l.source, now_ms: l.now_ms },
            Err(e) => Warmup::Unavailable(format!("the past candles did not decode: {e}")),
        },
        w::PastCandles::Unavailable(why) => Warmup::Unavailable(why),
    }
}

/// The host's side, for the glue's tests.
#[cfg(test)]
pub(crate) fn warmup_to_wire(value: &Warmup) -> w::PastCandles {
    use super::dec;
    match value {
        Warmup::Loaded { candles, source, now_ms } => w::PastCandles::Loaded(w::LoadedCandles {
            candles: candles
                .iter()
                .map(|c| w::Candle {
                    open_ms: c.open_ms,
                    open: dec(c.open),
                    high: dec(c.high),
                    low: dec(c.low),
                    close: dec(c.close),
                    volume: dec(c.volume),
                })
                .collect(),
            source: source.clone(),
            now_ms: *now_ms,
        }),
        Warmup::Unavailable(why) => w::PastCandles::Unavailable(why.clone()),
    }
}
