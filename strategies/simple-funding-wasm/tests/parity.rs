//! The port against the built-in: every call the built-in's own tests made to its decision core
//! (`parity/fixtures/simple-funding/`), replayed on the port's `decide`, `plan`, `assess`,
//! `fillable_within`, `fills_within` and `trailing_median`; the outputs must be the same.

use std::path::Path;

use bb_strategy_parity::simple_funding::{Cores, replay_all};
use bb_strategy_sdk::{Decimal, FundingSample, OrderBook, Side};
use bb_strategy_simple_funding_wasm::cost::{assess, fillable_within, fills_within};
use bb_strategy_simple_funding_wasm::decide::{MarketView, Params, SymbolView, decide};
use bb_strategy_simple_funding_wasm::plan::{Action, Caps, Row, plan};
use bb_strategy_simple_funding_wasm::signal::trailing_median;
use serde_json::{Value, json};

fn dec(v: &Value) -> Result<Decimal, String> {
    v.as_str()
        .ok_or_else(|| format!("not a decimal: {v}"))?
        .parse()
        .map_err(|e| format!("{v}: {e}"))
}

fn opt(v: &Value) -> Result<Option<Decimal>, String> {
    if v.is_null() { Ok(None) } else { dec(v).map(Some) }
}

fn flag(v: &Value) -> Result<bool, String> {
    v.as_bool().ok_or_else(|| format!("not a flag: {v}"))
}

fn count(v: &Value) -> Result<usize, String> {
    v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| format!("not a count: {v}"))
}

fn text(v: &Value) -> Result<String, String> {
    v.as_str().map(str::to_owned).ok_or_else(|| format!("not text: {v}"))
}

fn list(v: &Value) -> Result<&Vec<Value>, String> {
    v.as_array().ok_or_else(|| format!("not a list: {v}"))
}

fn book(v: &Value) -> Result<OrderBook, String> {
    let side = |v: &Value| -> Result<_, String> {
        v.as_object()
            .ok_or("book side")?
            .iter()
            .map(|(p, q)| Ok((p.parse::<Decimal>().map_err(|e| e.to_string())?, dec(q)?)))
            .collect()
    };
    Ok(OrderBook {
        bids: side(&v["bids"])?,
        asks: side(&v["asks"])?,
        last_update_id: v["last_update_id"].as_u64().unwrap_or_default(),
    })
}

fn view(v: &Value) -> Result<MarketView, String> {
    let p = &v["params"];
    let symbols = list(&v["symbols"])?
        .iter()
        .map(|s| {
            Ok(SymbolView {
                symbol: text(&s["symbol"])?,
                signal_8h: opt(&s["signal_8h"])?,
                round_trip_cost: opt(&s["round_trip_cost"])?,
                mark: dec(&s["mark"])?,
                oi_usd: opt(&s["oi_usd"])?,
                depth_usd: opt(&s["depth_usd"])?,
                fresh: flag(&s["fresh"])?,
                held_usd: dec(&s["held_usd"])?,
                entry_cost: opt(&s["entry_cost"])?,
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(MarketView {
        symbols,
        equity: opt(&v["equity"])?,
        foreign_gross_usd: dec(&v["foreign_gross_usd"])?,
        params: Params {
            notional_per_symbol: dec(&p["notional_per_symbol"])?,
            top_k: count(&p["top_k"])?,
            max_positions: count(&p["max_positions"])?,
            max_net_pct: dec(&p["max_net_pct"])?,
            max_oi_pct: dec(&p["max_oi_pct"])?,
            max_leverage: dec(&p["max_leverage"])?,
            expected_hold_hours: p["expected_hold_hours"].as_u64().ok_or("hold hours")?,
            min_depth_usd: dec(&p["min_depth_usd"])?,
        },
    })
}

fn run_decide(line: &Value) -> Result<Value, String> {
    let targets = decide(&view(&line["view"])?);
    Ok(Value::Array(
        targets.iter().map(|t| json!([t.symbol, t.notional_usd.to_string()])).collect(),
    ))
}

fn run_plan(line: &Value) -> Result<Value, String> {
    let rows = list(&line["rows"])?
        .iter()
        .map(|r| {
            Ok(Row {
                symbol: text(&r["symbol"])?,
                mark: dec(&r["mark"])?,
                held_base: dec(&r["held_base"])?,
                step: opt(&r["step"])?,
                oi_base: opt(&r["oi_base"])?,
                fresh: flag(&r["fresh"])?,
                target_usd: dec(&r["target_usd"])?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let c = &line["caps"];
    let caps = Caps {
        notional_per_symbol: dec(&c["notional_per_symbol"])?,
        max_positions: count(&c["max_positions"])?,
        max_leverage: dec(&c["max_leverage"])?,
        max_oi_pct: dec(&c["max_oi_pct"])?,
    };
    let actions = plan(&rows, opt(&line["equity"])?, dec(&line["foreign_gross_usd"])?, &caps);
    let side = |a: &Action| if a.side == Side::Buy { "buy" } else { "sell" };
    Ok(Value::Array(
        actions
            .iter()
            .map(|a| {
                json!({"symbol": a.symbol, "side": side(a), "qty": a.qty.to_string(),
                       "reduce_only": a.reduce_only})
            })
            .collect(),
    ))
}

fn run_assess(line: &Value) -> Result<Value, String> {
    let cost = assess(&book(&line["book"])?, opt(&line["fee_bps"])?, dec(&line["notional"])?);
    Ok(json!({"round_trip": cost.round_trip.map(|d| d.to_string()),
              "depth_usd": cost.depth_usd.map(|d| d.to_string())}))
}

fn run_fillable(line: &Value) -> Result<Value, String> {
    Ok(json!(fillable_within(
        &book(&line["book"])?,
        dec(&line["notional"])?,
        dec(&line["slippage_bps"])?
    )))
}

fn run_fills(line: &Value) -> Result<Value, String> {
    Ok(json!(fills_within(
        &book(&line["book"])?,
        flag(&line["buy"])?,
        dec(&line["qty"])?,
        dec(&line["limit"])?
    )))
}

fn run_signal(line: &Value) -> Result<Value, String> {
    let samples = list(&line["samples"])?
        .iter()
        .map(|s| Ok(FundingSample { time_ms: s[0].as_i64().ok_or("time")?, rate_8h: dec(&s[1])? }))
        .collect::<Result<Vec<_>, String>>()?;
    let now = line["now_ms"].as_i64().ok_or("now")?;
    let window = line["window_hours"].as_u64().ok_or("window")?;
    Ok(json!(trailing_median(&samples, now, window).map(|d| d.to_string())))
}

fn dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/simple-funding")
}

const CORES: Cores<'static> = Cores {
    decide: &run_decide,
    plan: &run_plan,
    assess: &run_assess,
    fillable: &run_fillable,
    fills: &run_fills,
    signal: &run_signal,
};

#[test]
fn the_port_decides_every_recorded_call_as_the_built_in_did() {
    let reports = replay_all(&dir(), &CORES).unwrap_or_else(|e| panic!("{e}"));
    let mut by_kind = [0usize; 6];
    for r in &reports {
        for (sum, n) in by_kind.iter_mut().zip(r.calls) {
            *sum += n;
        }
    }
    eprintln!(
        "fixtures {}, calls [decide, plan, assess, fillable, fills, signal] {by_kind:?}",
        reports.len()
    );
    assert!(by_kind.iter().all(|n| *n > 0), "every kind is replayed: {by_kind:?}");
}

/// The replay is not vacuous: a port whose entry hurdle differs fails it.
#[test]
fn a_port_that_decides_differently_fails_the_replay() {
    let looser = |line: &Value| -> Result<Value, String> {
        let mut v = view(&line["view"])?;
        v.params.expected_hold_hours += 100;
        let targets = decide(&v);
        Ok(Value::Array(
            targets.iter().map(|t| json!([t.symbol, t.notional_usd.to_string()])).collect(),
        ))
    };
    let cores = Cores { decide: &looser, ..CORES };
    let e = replay_all(&dir(), &cores).unwrap_err();
    assert!(e.contains("the built-in returned"), "{e}");
}
