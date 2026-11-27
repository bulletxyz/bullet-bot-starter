# simple-funding: Bullet-only funding carry

A dollar-neutral basket across many Bullet markets. Settings: `[strategies.simple-funding]`
Bullet is the only venue, so it needs a Bullet
key and nothing else. It has no mode of its own: the config document's is the run's (paper
until the owner's signed go-live).

## The risk, plainly

**Dollar-neutral is not price-neutral.** Funding arb (`funding-arb`) holds the same asset
long on one venue and short on another, so price cancels. Here every leg is a different
asset: the short book and the long book are equal in USD, but they move differently. The
basket earns the funding spread and takes the *relative* move between its legs, which can be
larger than the carry. Equity and commodity perps (TSLA, NVDA, GOLD, ...) also gap when their
underlying market is closed (weekends), and Bullet's price follows the next open. Neutrality
is set at entry only; as prices move the books drift apart (a replay peaked at 16% of gross).
`max_net_pct` stops *adding* to the heavy side; it does not rebalance by trimming.

**The replay below is optimistic and is not a test of the bot.** It runs `decide` alone: no `plan`
clamps, no IOC cap or partial fills (legs fill whole), no daily-loss stop, margin guard, pair
entry or imbalance watchdog, today's book and fees for every hour, and prices from another venue.
Its worst drawdown (52.85 USD, on ~617 USD of gross) is above any loss stop the shipped defaults
allow: on that window the stop would have fired. Read it as "was there carry", never as P&L. The
defaults are deliberately small (below) and **not a safe yield out of the box**.

Other things to know:

- **Defaults are conservative** (`config.rs`): 50 USD legs, `top_k` 2, `max_positions` 4 (at most
  200 USD gross), `max_leverage` 1, `max_net_pct` 0.15. **A live document is refused** (at the proposal
  and at the build) unless `daily_loss_usd` is set explicitly (positive, at most
  `max_positions x notional_per_symbol`; there is no live default) and a top-level `state_dir` is
  set (`SimpleFundingConfig::check_live`, and the bot's build). Paper
  without a stop assumes `DEFAULT_DAILY_LOSS_USD` (20).
- Use a **dedicated Bullet account**. Every position in a traded market is treated as the
  bot's: `decide` keeps or exits it, and shutdown flattens it. Positions in markets outside
  the universe (`only` / `exclude`) are never touched but count against `max_leverage`
  (valued at the mark the venue's unrealised PnL implies, not the entry price). A **live start
  is refused** when the account already holds a universe position the bot did not open
  (`adopt_existing_positions = false`, the default; a Limit-tier setting, so only the owner
  loosens it): going live from paper could otherwise sell a manual position at the first
  weak tick. The bot's own symbols are recorded in `day_state.json` (written before the opening
  order goes out) so its own restart is not refused. Without a `state_dir` nothing is
  recorded and every restart over open positions needs the setting. On paper its day state is
  the paper account's (kept by the bot): never the
  live `day_state.json`, and a paper rebuild knows the paper positions it carries as its own.
- The `daily_loss_usd` baseline (the equity at the first read of the UTC day), the halt and the
  bot's own symbols are saved as the `day_state` document (the strategy asks `Persist` to write it, acked before an
  opening order; the bot stores it and hands the saved record to the start). A crash, restart or rebuild
  resumes the same baseline and stays halted until the next UTC day or `Controls::resume()`;
  the day key is **ordered**: a clock that steps back, or a saved day ahead of the clock, neither
  resets the baseline nor lifts the halt (`roll_day` rolls only on `index > saved`);
  an unreadable file refuses the start, an unwritable one blocks entries. The baseline is plain
  account equity: the venue read exposes no deposits or withdrawals, so a deposit raises the
  loss needed to trip the stop and a withdrawal can trip it. Do not move funds mid-day.
  Several simple-funding instances on one state dir would share the file.
- Margin: every tick reads `Broker::margin_ratio`. At `LIQ_RATIO_FLATTEN` (0.8, `limits.rs`) it
  flattens and halts like the loss stop. An unreported or unreadable ratio blocks entries (it
  never trims a position on a missing number).
- Entries go in as **short/long pairs**: the second leg is sized to what the first filled, and
  the first is trimmed or closed by whatever the second leaves short (a rejected or clipped
  sibling never leaves a one-sided book).
- **An ambiguous submit is possibly filled.** A transport error, timeout or empty answer
  (`Sent::Ambiguous`; only a local `Strategy`/`Config` refusal counts as never sent) is read back
  from the venue's position at once. A leg that shows is treated as filled (the pair carries on);
  one that does not is held as `Pending` and judged after `AMBIGUOUS_SETTLE_SECS` against the
  venue's position: legs are matched in USD, anything one-sided beyond `PAIR_TOLERANCE` is
  unwound. No entry opens while a commit is unsettled.
- **An unwind that fails stays outstanding.** Accepted is not closed: each attempt is read back,
  and a leg still open is retried with doubling backoff (`UNWIND_BACKOFF_*`, wider cross each
  time), alerted through the notifier (`simple_funding_unwind_<symbol>`, escalating after
  `UNWIND_ESCALATE_AFTER`), and **no new pair opens while any unwind is outstanding**. `status`
  shows `unwinds`, `pending_commits`, `entries_blocked`.
- **Every IOC carries a cap**: the touch +/- `slippage_bps x mult`, never beyond
  `MAX_IOC_BPS_FROM_MID` (200 bp) of the book's mid however far a flatten retry widens. An entry
  needs a live REST book, two-sided, uncrossed, with its mid within that distance of the mark; a
  cached book is not a cap. A close with no live book falls back to a recent cached book, then a
  recent mark, then the position's entry price, capped around that reference. If the books sit past `max_net_pct` for
  `IMBALANCE_TICKS` ticks (a leg that drifted, a failed unwind) the lowest-ranked leg of the
  heavy side is exited. This does trim price drift beyond `max_net_pct`.
- An opening order is only built when its size fits inside the IOC cap (touch +/-
  `slippage_bps`) on the live book, and `decide` sees thin depth beyond that cap as no depth: the
  cost is judged on the walk the order can actually complete.
- Only the owner can lift a halt early: `Controls::resume()` on the actor handle. It is not
  wired to MCP, Telegram or the supervisor.
- **Hold and resume** (pause, the account kill switch): the supervisor's `Hold` runs `wind_down`
  (flatten), `Resume` builds a **new** actor (`launch` -> `build_simple_funding`) over the same
  `state_dir`, so the baseline and a persisted halt are restored and a resume does **not** lift
  a loss stop. A live start over positions (`recovering`: a hold whose flatten failed, a crash)
  allows no entry until a tick reads the book balanced, and sheds a lopsided one at once instead
  of waiting `IMBALANCE_TICKS`. Outstanding unwinds are in memory only; the venue's positions
  are what a restart rebuilds them from.
- A signal is the **median** of *settled* funding over the window (one spike among floor
  settlements does not move it: Bullet has had 3%/8h, which would move a 24h mean by 0.125%).
  A settlement beyond 5%/8h (`limits::MAX_FUNDING_RATE_8H`) is dropped on its own, not the whole
  signal; too few left is no signal. This cap is separate from the funding-arb executor's
  (0.5%/8h), which is unchanged.
- Taker fees on Bullet read 0 bp at every tier at the time of the replay below; the cost is
  then the book walk (half spread in and out). A fee change shows up in `taker_fee_bps`
  and the hurdle follows.
- The exit hurdle hangs off the **cost paid at entry** (`SymState::entry_cost`, the round trip
  assessed on the book the position was opened on), not today's book: a spread that widens for a
  minute does not exit a pair that a narrow book re-enters. After a restart it is unknown and
  falls back to today's round trip.

## Rules

- **Signal**: `trailing_median` (`bb_strategy_support::funding`) of the settled `rate_8h` over
  `signal_window_hours`, from `Broker::funding_history`. `None` below 3/4 coverage. The live
  `MarkPriceUpdate.funding_rate` is display only.
- **Entry** (flat symbol): `|signal| / 8 x expected_hold_hours > round_trip_cost`, fresh data,
  book depth >= `min_depth_usd` on the thinner side, OI known, the leg >= half of
  `notional_per_symbol` after the `max_oi_pct` clip. **Exit** (held): signal flips sign (or is
  0), or carry falls under half the cost paid at entry. Between the two a held leg is kept (hysteresis).
  Unknown signal or cost blocks an entry, never an exit.
- **Basket**: short the top `top_k` positive signals, long the top `top_k` negative ones, equal
  USD per leg, interleaved so both sides grow together; `max_positions`; leverage
  (`max_leverage x equity - foreign - held gross`); the long and short books within
  `max_net_pct` of each other (a one-sided basket never opens). Held legs are never displaced.
- **Cost**: `(2 x taker fee + walk in + walk out) / 10_000` (`cost.rs`, reusing
  `bb_strategy_support::funding::walk_slip_bps`).

## Where things are

- `src/decide.rs`: the pure `decide(MarketView) -> Vec<Target>` (signed USD per symbol). No
  I/O. Tests include property tests: leg size and OI caps, top_k, max_positions, leverage,
  neutrality bound, unknown signal never moves a position, no reversal.
- `src/plan.rs`: the targets clamped again before any order (leg size, OI, leverage room,
  max_positions, one new symbol per side per tick, stale or unknown inputs, size step).
  `decide` can be wrong; this cannot exceed a cap.
- `src/actor.rs`: the strategy's flows (`SimpleFundingActor`). Every tick: read
  the venue's positions and equity (a failed read **trades nothing**, never reads as flat),
  refresh fees / settled funding / OI (bounded per tick), `decide`, `plan`, place IOC
  `OrderType::Market` orders at the REST touch +/- `slippage_bps` (capped, see above), reduce-only
  for exits. It has no paper logic. A drop of equity past `daily_loss_usd` since the UTC day start
  flattens and halts until the next UTC day or `Controls::resume()` (off behind the strategy host,
  whose stop is the one). `stop` flattens everything in the universe, retrying with a wider cross,
  and fails if positions remain.
- `src/adapter.rs`: the flows as a `StrategyAdapter` (`SimpleFunding`) on ticks, marks and books, one
  flow at a time; a failed start or stop asks `Shutdown`. `src/adapter/io.rs`: the poller with no runtime, and
  the venue calls as `Intent::Request` / `Event::Answer`.
- `src/config.rs`: `SimpleFundingConfig`, the one `PARAM_SPECS` (sizes and limits are
  `Tier::Limit` with `Loosening::Up`, `min_depth_usd` `Loosening::Down`)
  and `settings()`.


Money-safety rules apply to this crate (root AGENTS.md): a change to `decide`, `plan`, the
loss stop, the flatten or the limits needs a failing test first. Do not edit
funding-arb's executor; this crate shares only the pure helpers in `bb-strategy-support`.

## Replay check (in the bot repository)

The ignored `live_backtest` test (which needs the network)
replays `decide` hour by hour over the last 15 days with the config the table was taken with
(100 USD legs, 3 per side, 8 positions, 2x: no longer the shipped defaults),
with public reads only: Bullet `fundingRate`, `depth`, `openInterest`, `exchangeInfo`. Bullet
has no klines endpoint, so hourly prices come from Binance futures (24 markets) or
Hyperliquid (GOLD, SILVER, BRENTOIL); WTIOIL, US100 and US500 have no price series and are
out. Costs use today's book for every hour; legs fill in full.

Run on 2026-11-02 (361 hours, 27 markets), USD (taken with the mean signal; `decide` only, see the
warning at the top):

| | |
|---|---|
| entries / exits | 8 / 3 |
| average gross / peak gross / peak \|net\| | 617 / 670 / 105 |
| carry earned | +14.56 |
| basket drift (price) | -0.70 |
| round-trip costs | 1.23 |
| net | +12.62 |
| max drawdown of the net curve | 52.85 |
| carry / average gross, annualised | 57% |

Carry beat the costs (14.6 vs 1.2) and the net drift (-0.7) in this window, but drift was not
small: per symbol it ranged from -18.3 (TSLA) to +6.1 (LIT), and hourly drift had sd 2.2 USD.
The worst peak-to-trough was 3.6x the whole 15 days of carry. One window of 8 entries cannot
say the sign of drift is stable, so treat the carry as likely smaller than the noise on a short
horizon and size `daily_loss_usd` accordingly. Most Bullet markets sit on the +0.01%/8h
interest floor, which pays shorts only; the basket needs the few markets with negative
funding (RAY, LIT, UNITREE, TIA here) to have a long side at all.

## Behind the strategy host (production build)

The bot runs simple-funding behind its strategy host as a plain `StrategyAdapter` (`SimpleFunding`: its flows run
on a std-only poller over `Intent::Request` / `Event::Answer`, `src/adapter/io.rs`, `src/adapter.rs`; alerts are
`Intent::Notify`), with its one exchange, Bullet, holding the whole universe as its markets. So, on top of the
rules above:
- every order passes the host's order path (the hard limits, the account's halt and notional cap, which counts the
  universe's gross) and the stale-price stop;
- the host's daily loss stop is the one stop (its limit takes this strategy's `daily_loss_usd`): a trip stops new
  orders, cancels each market (never `*`) and stands the instance down, and this strategy's own stop flattens; the
  margin halt stays its own;
- paper is the shared paper broker: the code trades it as a venue, margin is the venue's, the account value the
  paper account's, and nothing reaches the venue;
- a failed write, a timeout or a missing venue result is unknown to the host and to the code (an error answer),
  never a refusal; the strategy's ambiguous-leg rules handle it.
The tests that run the full host, and the recorded transcripts of this strategy's flows, live in the bot.
