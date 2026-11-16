# grid

Futures grid on Bullet (V1). `GridStrategy` (`src/adapter.rs`) is a `bb_strategy_sdk::StrategyAdapter`: it holds only
the level state and returns `Intent`s; the bot's strategy host places them and owns the inventory, the clock, the book,
the venue reads, paper and the snapshot. The bot reads the price history when the range or the levels are derived: its
build passes it (`with_history`), or its host loads the daily and hourly candles the grid lists in `histories()` (a load
that fails refuses the start with the host's reason), and passes the hard limits (`limits.max_position`,
`limits.max_notional_usd`, live only; `View::limits` when its host shows them, else the build's `with_limits`) and the
daily loss stop (`View::stop`, or the build's `with_day_stop`). The mode is the bot's (paper until the owner's signed
go-live), as for every strategy.

The candles: Binance's USDT-M perp first. It refuses US addresses (HTTP 451; 403 too), and many bots run in US
regions, so on a block or a connection failure the bot reads the spot candles of the same symbol from Binance's
public market-data mirror (spot volatility is close enough to size a range). The start is refused, naming both,
only when both fail. Bullet has no REST candles (only a WS kline topic).

**Money-safety code**: every change to `plan.rs`, `adapter.rs` and the files under `adapter/`, `levels.rs` or
`derive.rs` needs a failing test first, then the fix.

## The owner's three inputs and what is derived

- `amount_usd` (default $100; the dollars), `direction` (`neutral` default, `long`, `short`), `range` (`tight`,
  `normal` default, `wide`). `lower_price`, `upper_price`, `num_levels`, `order_size`, `anchor_price` override the
  derived values for an advanced owner; with the range and the levels all given no history is read. A wider
  `range` loosens (it moves the hard stop out): past `normal` it is the owner's to sign, as a wider
  `lower_price`/`upper_price` (each within [1e-8, 1e9]) or a higher `stop_beyond` is
  (`a_wider_range_preset_is_a_loosening_the_owner_signs`).
- `derive.rs` (Binance's method): range = mid ± clamp(3σ of daily close-to-close returns over the lookback),
  lookback Tight 7 d / Normal 30 d / Wide 90 d, the half-width held to Tight 2-29 %, Normal 3-47 %, Wide 4-60 % of
  the mid. Step = max(0.77 × ATR(14) on 1 h candles, 5 ticks, 0.1 % of the mid), up to the tick. Levels =
  min(range / step + 1, dollars ÷ (lot × mid) + 1, `MAX_LEVELS` = 50: half of Bullet's 100 open orders a market,
  and one start batch far under the path's 600 orders a minute). Arithmetic spacing, equal size per level.
- `plan.rs`: the dollars are `amount_usd`, or `limits.max_notional_usd` when lower (the real-money deposit sets
  it). The size per level makes the worst case equal the dollars, down to the lot; levels are dropped until each
  order is at least one lot (`MIN_LEVELS` = 4). The level nearest the anchor (the mid at start) within half a
  spacing is left empty (an anchor exactly halfway empties none). A venue tick or lot of zero, or one too small
  to divide the prices or sizes by, is refused, never a panic
  (`a_zero_or_vanishing_tick_or_lot_is_refused_not_a_panic`).

## Mechanics

- Neutral starts flat: buys below the mid, sells above. A filled buy re-arms a sell one level up; a filled sell a
  buy one level down. Every level rests (no open-order window: resting depth for takers).
- Long / short: one IOC at start, priced by the shared IOC cap (`capped_ioc_price`, 20 bps past
  the touch, never past 200 bps of the mid), sized as the levels on the far side (long: above the mid). A refused
  starting IOC, or one the host never sent (`Failed`), stops the bot (`Intent::Shutdown`). A long grid never sells
  into a net short and a short one never buys into a net long (no hard stop guards that side): its closing levels go
  out only as far as the position it holds, none at start, the rest on the first tick after the IOC fills.
- A level whose `Place` is in flight (sent, unanswered) is not placed again and counts toward the caps; an order
  answered after its level let go (a pause, a stop) is cancelled. A level whose `Place` was acked or left unknown
  is freed only by a read that started the settle margin (2 s) later: Bullet's REST can lag an acked placement.
  An order the grid let go of before it saw it gone (a pause's cancel, a read that omitted it) is cancelled again by
  a read past the margin that still lists it, and only such an order: never the owner's (`adapter/orders/stray.rs`).
- Post-only (default): a level that would cross goes at the touch on its own side, not dropped.
- A start on an account already holding the market's position is refused: the grid starts flat. Except a start
  that takes over a kept shutdown (`instance.on_shutdown` `keep_position`, the default, or `keep`;
  `adapter/reclaim.rs`): the kept position is its own, its kept orders are cancelled and re-placed under the
  per-side cap, no starting IOC goes out, and under the daily loss stop it stops and closes at once. A stopped grid
  closes at a kept shutdown (`may_keep`).

## Safety rules (tests in `adapter_tests.rs`, the `adapter_*_tests.rs` beside it and `plan_tests.rs`)

1. Worst case (every risk-side level filled) must fit `amount_usd`, the grid's own `max_position` and the hard
   limits, or the start is refused (`a_worst_case_past_the_hard_limits_or_the_dollars_is_refused`,
   `start_refuses_a_worst_case_past_the_hard_limits`).
2. Hard stop `stop_beyond` (default 0.5) range widths past the losing edge (below for long and neutral's long
   side, above for short and neutral's short side), checked on every book and tick: cancel its own orders
   (`CancelOwn`: a normal stop, so the owner's orders on the market stay; the daily loss stop is a safety stop and
   cancels them all), close with
   reduce-only IOCs (the path holds them to the venue's position), stop; every tick closes again while a position
   is open, wider each time (`past_the_hard_stop_it_cancels_its_own_orders_closes_reduce_only_and_stops`,
   `a_failed_daily_loss_cancel_is_asked_again_market_wide`,
   `the_short_sides_hard_stop_is_above_the_range_and_buys_back`,
   `the_hard_stops_close_goes_through_the_path_sized_to_the_venue_position`). A range so wide that the stop below
   it is at or under zero refuses the start, since that stop never fires: a `wide` range on a very volatile market
   refuses rather than run without it (`a_hard_stop_at_or_below_zero_refuses_the_start`).
3. Out of range on the profitable side (long above, short below): the opening orders are cancelled until the price
   is back; on the losing side the grid holds until the hard stop; no re-centring
   (`a_long_grid_above_its_range_cancels_its_buys_and_resumes_back_inside`,
   `below_the_range_but_above_the_stop_the_grid_holds`).
4. The trend filter cancels the opening orders only (the closing ones, against the position and nearest the
   price, stay); the daily loss stop (the bot's loss stop halts new orders and tells the grid) flattens and
   stops like the hard stop, as rsi-momentum's does
   (`a_trend_filter_trip_cancels_the_opening_orders_and_keeps_the_closing_ones`,
   `the_daily_loss_stop_flattens_and_stops_the_grid`).
   With a trend filter the host first loads three slow windows of 1m candles (Binance's) and folds their closes
   into its EMAs at their close times (`adapter/warm.rs`, `GridState::warm_trend`), so a market already trending
   into the start holds the opening orders back from the first placement; a failed or short load starts the
   filter at the start's mid, as before, and the report's `history` says so
   (the bot's `src/builtin/grid_host_history_tests.rs`).
5. `report()` shows `pnl.realized`, `pnl.unrealized` and `pnl.total` of the whole position from the host's view
   (`the_report_shows_realized_and_unrealized_of_the_whole_position`).

A stopped grid stays stopped for the run. Its hard stop also holds the instance for safety (the grid asks the bot for a
hold, the build's `with_hard_stop_hold` or else `Intent::StandDown`, and raises `grid_hard_stop` to say why), so a
restart starts held, never a new grid at the then-current mid; the owner's resume starts one, whose start resolves the
alert (`the_hard_stop_holds_the_instance_and_says_why`, `the_hard_stop_stands_the_instance_down_through_the_port`).
Until the build's hold goes, only a start with one resolves the alert.

## Files

- `derive.rs`, `plan.rs`, `levels.rs` (levels, fills, re-arm, closing vs opening, pauses), `levels/ema.rs`,
  `config.rs`: no I/O, no clock. Time is `now_ms: u64` from `view.now_ms`.
- `adapter.rs`: `GridStrategy`, its builders and the state it keeps; `adapter/port.rs`: the host's calls;
  `adapter/events.rs`: fills, order updates, books and ticks; `adapter/orders/`: placement, the reconcile of what a
  read shows, the strays; `adapter/stops/`: the hard and daily loss stops and their cancel (`pull.rs`);
  `adapter/pauses.rs`: out of range and the trend filter; `adapter/warm.rs`, `adapter/reclaim.rs`.
- `describe.rs`: `Describe` (with its profile: the owner's page, the `sol-grid` preset) and `FromSettings`, how the
  bot names and builds it.
- The grid keeps issuing client ids for its levels (`ClientIdIssuer`): `Event::Outcome` and `Event::Fill` find a
  level by client id.
- Tests: `adapter_tests.rs` (call `on_event` on a hand-built `View`; by behaviour in `adapter_direction_tests.rs`,
  `adapter_orders_tests.rs`, `adapter_pauses_tests.rs`, `adapter_stops_tests.rs`), `plan_tests.rs`, `derive_tests.rs`,
  `levels_tests.rs`, `config_tests.rs`; `adapter_keep_tests.rs`: the reclaim (`adapter/reclaim.rs`) and the kept
  shutdown; `tests/parity.rs` replays the calls the strategy's tests made under the bot's host
  (`parity/fixtures/grid/`), so its intents must stay what they were. The tests that run the full host (order path,
  paper, stops) live in the bot. Edited, not re-recorded, for the settle margin on an acked level (a read within 2 s of
  the acks no longer frees them): `the_fallback_note_goes_once_the_filter_has_warmed_live` (its last tick places
  nothing, and the outcomes of the orders it no longer sends are gone),
  `a_tick_replaces_a_level_the_venue_no_longer_shows` (its tick, in the ms of the acks, places nothing) and
  `below_the_range_but_above_the_stop_the_grid_holds` (its headline counts the two held buys). Edited for the stand-down
  (a hard stop with no build hold now ends its round with `StandDown` and the `grid_hard_stop` alert; the bot's host
  tests ran without a hold, so none was recorded): the hard stop's round in
  `past_the_hard_stop_it_cancels_its_own_orders_closes_reduce_only_and_stops`,
  `the_short_sides_hard_stop_is_above_the_range_and_buys_back`,
  `the_hard_stops_close_goes_through_the_path_sized_to_the_venue_position`,
  `a_failed_cancel_all_is_asked_again_on_the_next_tick_until_it_lands`,
  `a_cancel_all_that_failed_as_a_whole_is_asked_again_too` and
  `the_owner_is_told_after_five_failed_cancel_alls_in_a_row_and_the_grid_keeps_trying`.
