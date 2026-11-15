# simple-funding-wasm

A dollar-neutral funding carry basket across Bullet markets: short the markets whose settled
funding pays shorts, long the ones that pay longs (`top_k` per side), equal USD per leg, entered as
short/long pairs, never more than the leverage and open-interest caps allow, long and short books
kept within `max_net_pct` of each other.

**Dollar-neutral is not price-neutral.** Every leg is a different asset; the basket earns the
funding spread and takes the relative price move between its legs, which can be larger than the
carry (equity and commodity perps also gap when their market is closed). Defaults are small
(50 USD legs, 2 per side, 4 positions, 1x) and are not a safe yield. Start on paper.

## Use

1. Install it like any plugin ([docs/INSTALL.md](../../docs/INSTALL.md)) and have the owner sign its hash.
2. Give the leg `universe` (venue `bullet`) the markets to trade: `legs.universe.symbols`. The
   instance's own symbol is not traded. Use a dedicated Bullet account: every universe position
   is treated as this strategy's.
3. Settings are the built-in's (`notional_per_symbol`, `top_k`, `max_positions`, `max_net_pct`,
   `max_oi_pct`, `max_leverage`, `signal_window_hours`, `expected_hold_hours`, `slippage_bps`,
   `min_depth_usd`, `daily_loss_usd`, `stale_secs`, `adopt_existing_positions`, `exclude`, `only`).

## What is the same as the built-in

`decide` (the basket), `plan` (the caps again, as orders), the book-walk cost and depth, and the
median funding signal are the built-in's rules, and the parity tests replay every call the
built-in's own tests made to them (`parity/fixtures/simple-funding/`: 105 fixtures, 717 calls) and
require the same answers. The execution layer follows the built-in's rules too: positions and
equity read every tick (a failed read trades nothing), no entry on stale data or an unknown equity,
margin, fee, open interest or signal, IOC orders capped within 200 bp of a live book's mid, pair
entries with the second leg sized to what the first filled, unwind of a one-sided leg with backoff
(no new pair while one is outstanding), ambiguous submits read back from the venue before any
further entry, a lopsided book shedding its weakest heavy leg, a daily loss stop and a margin
flatten.

## What it cannot do (the `bb:strategy@0.5.0` world has no place for it)

- **No saved state.** The built-in saves the day's loss baseline, a halt and the symbols it
  opened. This keeps them in memory: a restart re-baselines the daily loss from the equity then,
  lifts a halt, and, finding positions it cannot know are its own, refuses to trade (a start over
  them fails) unless `adopt_existing_positions` is set. Do not rely on its daily loss stop across a
  restart; the bot's own limits still apply to every order.
- **No live/paper difference.** The built-in refuses a live run without an explicit
  `daily_loss_usd`; a plugin does not know its mode, so the default 20 USD applies always and a
  value above `max_positions x notional_per_symbol` (no stop at all) is refused.
- **No alerts, no owner resume.** The world has no notification intent; unwinds, ambiguous orders
  and halts are in the report fields. A halt lasts to the next UTC day.
- **Wind-down is one `Flatten`** per held market (the bot carries it out and retries); the
  built-in's five retries with a widening cross are the bot's here.
- **No presets and no featured/risk text:** the descriptor has no place for them.
- The execution layer is tested against a simulated venue (`src/tests/`), not replayed call for call
  against the built-in: it is a request/answer flow, so a byte replay would pin only the call order.
- The built-in's backtest (`tests/backtest.rs`, a replay of `decide` over 15 days of public data) is not
  ported; its views are not in the fixtures.

Bump `version` in `Cargo.toml` for any change.
