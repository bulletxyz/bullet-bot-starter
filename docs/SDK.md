# SDK cheat sheet

Everything a strategy touches, on one page. The source is `sdk/src/` (`adapter.rs`, `types.rs`,
`calls.rs`, `legs.rs`, `history.rs`, `describe.rs`, `testing.rs`); the template
(`templates/strategy/`) uses most of it.

Agents: clone the repo first (`git clone https://github.com/<owner>/bullet-bot-starter`, the
user's copy if they have one, else this one) and read `sdk/src/`, the template and the examples from
the checkout, not file by file over the web.

## The three traits

- `StrategyAdapter`: `start(view)` once before any event (an `Err` stops the build),
  `on_event(event, view)` for each event, `stop(view)` when the bot winds down (return
  `CancelAll`), `report()` for the owner's page. Optional: `history()` and `warm_up(warmup)` for past
  candles (below).
- `FromSettings::from_settings(json)`: build the strategy from the owner's settings (JSON text). An
  `Err` names what it can't use; the bot refuses the load.
- `Describe`: `NAME` (the folder's name: lowercase letters, digits, `-`, `_`), `VERSION` (use
  `env!("CARGO_PKG_VERSION")`) and `describe()`, which runs before the settings exist.

`export_strategy!(MyStrategy)` at the crate root makes it a component.

## `View`: what every call sees

| Field | What |
|---|---|
| `now_ms` | The bot's clock (Unix ms). The only clock a strategy has. |
| `symbol` | The market it trades. |
| `book` | `Option<&OrderBook>`: `bids` and `asks` (price to size), `best_bid()`, `best_ask()`, `midpoint()`. |
| `position`, `avg_entry`, `realized_pnl` | The bot's inventory from fills, seeded from the venue's position at start. Signed: short is negative. |
| `open_orders` | `Some` only on the view `start` gets and on a tick's first round when the venue read worked; `None` means unknown, never "no orders". |
| `positions` | The venue's positions from that same read, same freshness. `signed_position(positions, symbol)` gives one signed size. |
| `read_started_ms` | When that read began; `None` without a fresh read. |
| `reconnected` | The venue stream reconnected since the last tick: events may be missing, so reconcile. True on one tick. |
| `venue_halt`, `path_stop` | Why the venue, or the bot's own order path (its stop), refuses new orders now. While either is `Some`, don't quote. |
| `market` | `tick_size`, `step_size`, `maker_fee_bps`, `taker_fee_bps`, `fee_tier`, `venue_name` (each optional). |
| `legs` | Other venue accounts the strategy declared (`Descriptor::legs`); empty for one market. |

## `Event`: what happens

| Event | Carries |
|---|---|
| `Book` | Nothing: the new book is in `view.book`. |
| `Mark(price)` | The venue's mark price for the symbol. |
| `Tick` | Nothing. A tick's first round may carry a fresh `open_orders` / `positions` read. |
| `Fill(Trade)` | One fill: `client_id`, `order_id`, `side`, `price`, `quantity` (this fill only, so it may be part of the order), `timestamp`. |
| `Order(Order)` | An order update: `client_id`, `status` (`Open`, `PartiallyFilled`, `Filled`, `Cancelled`, `Rejected`), `quantity`, `filled_quantity`. |
| `Outcome { intent, result }` | The answer to one of your intents: `result.success`, `result.error`, `result.order_id` (`None`: not known yet). |
| `Unknown { intent, error }` | The call for a place or amend failed after it may have reached the venue: the order may rest or have filled. |
| `Answer { id, reply }` | The one answer to your `Request` with that `id`. |
| `Leg { leg, symbol, event }` | A book, mark, fill or order update on another leg. |

## `Intent`: what you ask for

- `Place(NewOrder)`: `symbol`, `side`, `order_type` (`Limit`, `PostOnly`, `Market`), `price`,
  `quantity`, `client_id`, `reduce_only`, `protect` (a stop and take-profit the venue holds for the
  position once it fills; paper sends none).
- `Cancel(CancelOrder)`: by venue `order_id`, or **by client id with `order_id` empty**
  (`order_id: String::new(), client_id: Some(id)`).
- `Amend(AmendOrder)`: a cancel and a new order in one.
- `CancelAll`: your own orders only, never the owner's.
- `Flatten { symbols }`: close the venue's position in each (reduce-only IOCs); refused on the
  owner's main account for the symbol it trades.
- `Request { id, leg, call }`: a venue read (`Positions`, `OpenOrders`, `Book`, `AccountEquity`,
  `MarginRatio`, `FundingHistory`, `OpenInterest`, `RefreshMetadata`), a batch write, or `Wait {
  until_ms }`. Answered once by `Event::Answer` with the same `id`; don't reuse an id in flight.
- `Leg { leg, intent }`: an intent on a declared leg (`TRADE_LEG`, `"trade"`, is the main one).
- `Shutdown`: never; the bot faults a plugin that sends it. To stop, cancel and send nothing more.

At most 64 intents a call.

## Patterns

- **Client ids.** Give every order one and match events by it. The bot takes printable ASCII of
  at most 64 bytes, so leave the strategy's name out. Start them with `view.now_ms` from `start`
  (the template: `<start ms>-<n>`) so a new run never repeats an id from an earlier one.
- **Partial fills.** A `Fill` is one piece. Add up `quantity` per client id; the order is done when
  the sum reaches its size, or when `Order` says `Filled`.
- **Cancel and replace.** Send `Cancel` by client id and mark the order as cancelling. The template
  and ladder treat the order as gone on its `Order` update (`Cancelled`), then place the new one;
  a fill can still come in for it until then.
- **`Unknown`.** Never place again as if refused: keep the order's slot until its events come, or
  until a fresh read that started after you sent it lists it (it rests) or leaves it out (gone).
  The template waits 30 s (`LIST_LAG_MS`); see `strategies/ladder/src/retry.rs` for more.
- **Freshness.** Act on `open_orders` / `positions` only when they are `Some`, and decide that an
  order is gone only from a read whose `read_started_ms` is after you sent it (plus a lag).
- **Position.** `view.position` is the bot's from fills; size closes on the venue's
  (`signed_position(view.positions?, view.symbol)`) when you have a fresh read.
- **Numbers.** `Decimal` and `RoundingStrategy` are both exported by the SDK: round a size down with
  `size.round_dp_with_strategy(8, RoundingStrategy::ToZero)`, no `rust_decimal` dependency needed.

## Settings and `describe()`

`Settings` is a serde struct (`#[serde(deny_unknown_fields)]`); `from_settings` checks it.
`describe()` returns a `Descriptor`: `summary`, `settings` (one `SettingSpec` each, from
`SettingSpec::decimal(path, default, range, help)`, `::int(...)`, `::flag(path, default, help)` or
`::choice(path, choices, default_index, help)`, then `.tier(Tier::Limit)`,
`.loosening(Loosening::Up)`, `.required()`), `settings_schema: schemars::schema_for!(Settings)`, and
`legs` (`LegNeed::new(id, venue)` per extra venue account). `Tier::Limit` with `Loosening::Up` means
raising it needs the owner's signature. Name settings as you like: the bot never reads one as its
own limit.

## Past candles

`history()` returns `Some(HistoryNeed { interval, want, need })` (`CandleInterval::M1` to `D1`, at
most 999 bars). Before `start`, `warm_up` gets `Warmup::Loaded { candles, .. }` (oldest first) or
`Warmup::Unavailable(why)`. See `strategies/sma/`.

## The report

`report()` returns your own type implementing `StrategyReport`: `headline()` (one line) and
`fields()` (`ReportField::number`, `::text`, `::flag`, `.unit(..)`): at most 128 fields, names 1-64
characters, texts at most 200, units at most 16, or the bot faults the plugin. Position and P&L are
the bot's: don't report them.

## Testing with `SimHost`

With the SDK's `testing` feature (a dev-dependency):

```rust
use bb_strategy_sdk::testing::{Market, SimHost, book, d, fill_of, outcome, placed};

let mut sim = SimHost::new(strategy, Market::new("BTC-USD", "99", "101", NOON));
let sent = sim.start()?;                 // every intent sent, outcomes fed back
let buy = placed(&sent)[0].clone();
sim.send(&fill_of(&buy));                // a full fill; it moves sim.market.position
sim.market.book = Some(book("101", "103", "10"));
let next = sim.send(&Event::Book);
```

`SimHost` answers every place (taken, unless `refuse` says why not), cancel and amend with an
`Outcome`, up to 3 rounds deep, cancels before amends before places; the views of those answer
rounds carry no `open_orders`. It sends no `Order` updates, `Unknown` or `Answer`: build those events
yourself (`outcome(&intent, Ok("id"))` or `Err("why")` makes an `Outcome`) and
`send` them. Set `Market` fields (`position`, `open_orders`, `read_started_ms`, `path_stop`, ...)
to shape the view.
