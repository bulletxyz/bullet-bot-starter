# rsi-momentum: RSI momentum on one Bullet market

Directional and unhedged. Bullet is the only venue. No `dry_run` of its own: the mode is the bot's (paper until
the owner's signed go-live).

It is a `StrategyAdapter` (`RsiMomentum`, `src/adapter.rs`): the strategy keeps its signal, entries, exits and stop
and returns them as intents. The bot's host owns the position (from fills; taken over from the venue after
the host's confirm ticks of disagreement), the order path's limits, the paper broker (the same one the grid uses),
the one daily loss stop every strategy has, and a wind-down that reads the venue, runs the strategy's close, then
the host's own close until the venue reads flat. The strategy keeps the venue's position as the host last read it
(`View::positions`, at start and each tick; unknown after a fill or a tick with no read) and sizes and ends its
closes on it. Under `keep_position` (the default) a process shutdown leaves the long or short open unless a close is
under way; the next start takes it over: the host seeds it and the stop and exit watch it as their own.

## Rules (the strategy's own)

- **Candles** (`indicator/candles.rs`): built from `Event::Mark` (the venue's mark price the host delivers),
  bucketed by the host's clock. A candle closes on the next mark in a later bucket. The first RSI
  needs `rsi_period + 1` closed candles, an entry one more (a cross).
- **Warm-up** (`history`, `warm_up` in `adapter.rs`): the host loads up to ten times `rsi_period + 1` past
  candles of the interval (Bullet's API has none, so the bot reads another venue's), and their closes seed the RSI,
  so it can trade at the first live close. The first live mark must fall in the candle the loaded ones end at
  (`meet_seam`); in a later one the warm-up is dropped and the RSI warms up from live marks, as after any gap. A
  failed, short or mismatched load warms up from live marks. The report's `history` says which.
- **Gaps**: buckets with no mark between two candles reset the RSI.
- **RSI** (`indicator/rsi.rs`): Wilder's smoothing.
- **Entry** (`decide::on_close`): flat, the RSI **crosses** above `long_above`, or below
  `short_below` with `allow_short`; none while `cooldown_candles` remain after an exit. Sized
  `notional_usd / mark`, capped at `max_position`, floored to the step. Needs no close under way,
  no entry in flight, flat, a fresh mark (`stale_secs`) and a live two-sided book whose mid is
  within 200 bp of the mark.
- **Exit**: at a candle close past `exit_band`; on **any mark** at `stop_loss_pct` against the entry.
- **Orders**: marketable IOC (`OrderType::Market`) at the touch +/- `slippage_bps`, never past
  `MAX_IOC_BPS_FROM_MID` of the mid (`bb_strategy_support::ioc::capped_ioc_price`). Closes are
  reduce-only, sized to the venue's position as last read, else the host's, cross
  `3 x slippage_bps x attempt` and repeat every tick; after `CLOSE_ATTEMPTS` it raises
  `rsi_momentum_close_failing` (`Intent::Notify`), which stays raised until the next start (the
  owner's resume) resolves it.
- **Live** needs an explicit `daily_loss_usd` and a state directory (`config.rs`); the bot gives the host
  `max_position` and `daily_loss_usd` (paper default `DEFAULT_DAILY_LOSS_USD`) as its limits (each names the limit it
  sets: `Setting::caps`).

## Behaviour that changed with the port (intentional)

- The daily loss stop, its day and its halt are the host's, not the strategy's.
- Paper is the host's paper broker (fills against the book), not fills at the touch.
- No entry gate on "equity unknown" of its own: the host's loss stop refuses new orders while the
  account value is unread (past two reads).
- A close is sized to the venue's position as the host last read it (each tick), not to a read made
  just before it; reduce-only still holds. The wind-down's close is the strategy's one IOC, then the
  host's close (at the touch capped 20 bp from mid).
- An ambiguous (timed-out) order is the order path's: counted as possibly landed, and a run of them
  stops new orders and alerts; the entry holds its slot meanwhile.
- The start no longer fails without a size step; entries wait for one.

## Where things are

- `src/indicator/candles.rs`, `src/indicator/rsi.rs`, `src/decide.rs`: pure, unit-tested (each `*_tests.rs` beside
  it). `src/adapter.rs`, `src/report.rs`;
  tests in `src/adapter_tests.rs` (venue-read entries in `src/adapter_venue_tests.rs`) and `tests/parity.rs`
  (the calls its host-run tests made, replayed). The tests that run the full host live in the bot.
- `src/limits.rs`: fixed safety constants. `src/config.rs`: `RsiMomentumConfig`, `PARAM_SPECS`, `settings()`.
  `src/describe.rs`: `Describe` (with its profile: the owner's page, the `btc-rsi-momentum`
  preset) and `FromSettings`, how the bot names and builds it.

Money-safety rules apply (root AGENTS.md): a change to `decide`, the adapter's closes or the IOC cap
needs a failing test first.
