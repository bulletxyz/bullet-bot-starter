# simple-mm: the bot's market maker

One bid and one ask on one Bullet market around a fair value, with optional inventory skew and volatility
widening. `MarketMaker` (`src/adapter.rs`) is a `bb_strategy_sdk::StrategyAdapter`: it keeps its two quotes and
returns intents. The bot's host owns the inventory, the order path and its gate (the order rate, the account's
caps), paper (its paper broker), every guard (daily loss, stale price, fast market, the reject breaker, the venue's
stale oracle, the reconcile of stray orders, the position resync), the owner's pause, the limits and the state.

**The name.** `simple-mm` is the folder, `Describe::NAME` and the crate's (`bb-strategy-simple-mm`): v51
instances (`"strategy": "simple-mm"`), their config sections (`[strategies.simple-mm]`) and the log prefixes carry
it.

**Money-safety code**: `src/cap.rs`, `src/pricing.rs` and `src/pricing/`, `src/params.rs`, `src/adapter.rs` and
the files under `src/adapter/` need a failing test first, then the fix.

## Pricing (`src/pricing.rs`, `src/pricing/fair_value.rs`)

- Quotes sit at the fair value ± `bid_spread_bps` / `ask_spread_bps`, plus `vol_widen_bps` a side while the
  Bullet mid's high-low range over `vol_window_secs` reaches `vol_threshold_bps` (`vol_widen_enabled`, off by
  default), shifted by the skew `-skew_bps × net / max_position` (the ratio held to ±1), then kept off the touch:
  a buy at least one tick under the best ask, a sell one over the best bid (no tick known: it joins the touch).
- The fair value is the Binance USD-M perp mid plus an EMA of the basis (Bullet mid − Binance mid) with
  `fair_value = "binance"` (the default), or the Bullet mid with `"bullet"`. It falls back to the Bullet mid,
  and the report says why (`fair_source`, `fair_fallback`, `basis_bps`), without a Binance mid fresher than
  `reference_stale_secs` (judged by when it was received), before the basis has averaged `BASIS_WARMUP_SECS`
  (60), or while the Binance fair value is more than `max_divergence_bps` from the Bullet mid. Once warm, a
  divergent sample is left out of the basis; after `BASIS_RELEARN_SECS` (300) of divergence the basis warms up
  again at the new level. A switch back to binance warms up from scratch. A Binance outage never stops quoting.

## Quoting (`src/adapter/quoting.rs`, `src/adapter/sync.rs`, `src/cap.rs`)

- A refresh on each book and tick once `refresh_secs` has passed (at once after a pull or the host's cancel),
  and one round at a time (the last one's answers all in).
- Each side's quote passes the market maker's own cap first (the host's gate rule, so it never asks for what the
  gate refuses): sized at most the position cap (rounded down to the step when clamped), dropped when its fill
  would hold more than `max_position` or `max_notional_usd` that way (each the lower of the section's and the
  host's live limit), or when its size is zero. A post-only quote that would cross is not sent.
- A side with no quote gets a place; a quote no longer wanted is cancelled; one whose size differs or whose price
  drifted `refresh_threshold_bps` or more is repriced. A reprice is one `Amend` when the replacement passes the
  cap with the old quote counted as filled (the venue skips the cancel of an order that no longer rests); near the
  cap, or for a quote with no venue id, the old quote is cancelled first and the replacement placed on the
  cancel's answer, re-capped at the position then, and not at all if a stop came meanwhile
  (`near_the_cap_a_reprice_cancels_first_and_places_on_the_answer`).
- A quote that may still rest but is no slot (the old quote of a refused amend, a failed cancel beside an amend,
  the old order of an amend left unknown) is a stray: the next refresh cancels it and places nothing until it is
  gone, by its cancel's answer, its terminal update, or a read
  (`a_failed_cancel_beside_its_amend_leaves_a_stray_that_blocks_placing`).
- A place left unknown holds its side (never placed again) until a read settles it; a read frees a slot or a
  stray it does not list only when it started `SETTLE_MS` (2 s) after the answer. The venue is read every 30 s,
  on a reconnect, and while a stray or an unknown place is held (`wants_venue`).
- While the host stops new orders (Bullet's `ExchangeView::path_stop`, `View::stop`, `ExchangeView::venue_halt`), while paused or closing,
  and without a two-sided book, nothing is quoted and what rests is pulled with one `CancelOwn`, asked again at
  most every 10 s while it fails. `stop` asks nothing: the host's wind-down cancels (or keeps) the quotes.

## Commands (`src/adapter/control.rs`)

- `SetSettings`: the live params (`BUILTIN_SPECS`, and the strategy's own `PARAMS` under `extra`), each changed
  one checked against its guarded range, or its hard range when the config store authorized it
  (`BoundsMode::Extended`); `order_size` against the venue's step and half the position cap. Applied on the next
  refresh; the reply names each change, old to new.
- `Pause` / `Resume`, when the host leaves them to the strategy: the pause pulls and replies once the pull is back;
  it is not saved (the host's pause is). `ClosePosition`: no quote from then on, the host's close (it cancels the
  symbol's orders first), and the reply worded from its `CloseReport`. `SetLimits`, the kill switch and staging
  are refused: they are the host's and the supervisor's.

## Settings (`src/settings.rs`, `src/settings/specs.rs`, `src/describe.rs`)

The section's keys are v51's, `deny_unknown_fields`. `dry_run`, `state_dir`, `max_inflight`, `guards` and
`max_position_fallback` are accepted and ignored: the mode and the state are the instance's, the rest the host's.
`max_position` and `daily_loss_usd` set the host's limits (`Setting::caps`). The profile has the bot's owner page,
the `btc-conservative` and `eth-balanced` presets, and the reference feed (`needs`).

## Behaviour against the bot's in-tree simple-mm (its rig fixtures, `tests/simple_mm_host`)

- A reprice is one `Amend`; near the cap the two-step replacement goes out in the same event, on the cancel's
  answer, not at the next refresh.
- Every pull is the market maker's own orders (`CancelOwn`), never the market's: a guard's market-wide cancel is
  the host's.
- A refused amend's old quote is cancelled again before anything is placed (it may still rest).
- The guards, the gate, the order rate, the account's checks, paper, the saved stops, the journal, the history
  and the `/status` keys are the host's; the pause is not saved by the strategy. Of the snapshot the market maker
  supplies only its quotes (`snapshot_extras`: its two slots, as the old one showed them), since the venue's last
  read lags a place or a cancel by a read.
- `paper_quoting_secs` counts any run's time quoting: the strategy cannot tell paper from live.
- The close's reply no longer says "on paper".

## Tests

Beside each file: `pricing/fair_value_tests.rs`, `pricing_tests.rs`, `cap_tests.rs`, `params_tests.rs`,
`settings_tests.rs`, `report_tests.rs`, `describe_tests.rs`; `adapter_tests.rs` (quoting, answers, reads, pulls)
and `adapter_control_tests.rs` (commands, the close) drive the strategy on hand-built views and `SimHost`;
`purity_tests.rs` fails if the crate names a clock, the history, a store or a log. The tests that run the full host
(the order path, paper, the guards, the golden replays) live in the bot.
