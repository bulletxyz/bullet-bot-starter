# bb-strategy-sdk changelog

## 0.7.0

A strategy sees every exchange it trades, each with its markets: the one main market and its legs are gone.
Bullet is one exchange like any other, and every strategy trades it. This is a new world, `bb:strategy@0.6.0`:
plugins built with 0.6 do not load on it, and plugins built with 0.7 need a bot that runs it.

**Breaking**
- `View` is `now_ms`, `exchanges: &[ExchangeView]`, `limits`, `stop`; `view.exchange(venue)` and
  `view.market(venue, symbol)` find one. `ExchangeView` holds the venue, its `markets` (`MarketView`: symbol,
  book, mark, funding rate, facts, position, average entry, realized P&L), `open_orders`, `positions`,
  `reconnected`, `venue_halt`, `path_stop` and `read_started_ms`; `venue_position(symbol)` reads one position.
- `Event::Market { exchange, symbol, event }` (`MarketEvent::{Book, Mark, Fill, Order}`) replaces `Book`, `Mark`,
  `Fill`, `Order` and `Leg`.
- `Intent::On { exchange, intent }` (`Intent::on(venue, ..)`) carries every order: `ExchangeIntent::{Place,
  Cancel, CancelAll(symbol), CancelOwn(symbol), Amend, Flatten(symbols)}`. `Intent::Leg`, `Intent::Flatten` and
  the top-level order intents are gone. `Intent::Request` names its `exchange`; `Intent::ClosePosition` names
  its `exchange` and `symbol`.
- `Descriptor::exchanges` (`ExchangeNeed::new(venue)`) replaces `legs`, and must list `BULLET`:
  `Descriptor::check` (and the component's `init`) refuses one without it, or with a venue twice. `LegView`,
  `LegNeed` and `TRADE_LEG` are gone.
- The owner page's `profile::ExchangeNeed` is now `ExchangeRole`.
- `testing::Market` holds `exchanges` (`testing::exchange`, `testing::market`); `fill_on(exchange, order)`.

## 0.6.0

The strategy port now carries what the built-in strategies need, so the bot registers them with no
per-strategy code. Everything added is native-only: the `bb:strategy` WIT world is unchanged. Plugins
built with 0.5 still load in a new bot, and plugins built with 0.6 still load in an older one.

**Added**
- `Describe::profile()`: a built-in's owner-page text, paper and loss-stop flags, exchanges, settings
  and presets.
- `Describe::needs()`: the feeds, the position resync and the flatten on wind-down a strategy asks for.
- `Strategy::histories()` and `warm_up_for(need, warmup)`: several candle intervals, one need each.
- `View::limits` (with `LimitsInForce::max_notional_usd`) and `View::stop`, the host's stop, typed.
- `Intent::StandDown(reason)`: the instance asks the host for a safety hold.
- `Setting::caps(HostLimit)`: a setting names the host limit it sets.

The new trait methods all have defaults, so a strategy that ignores them still compiles.

**Breaking for Rust source** (one-line fixes; compiled plugins are not affected)
- `Intent` has a new variant, `StandDown`. A `match` over every `Intent` without a `_` arm needs an
  arm for it.
- `Setting` has a new public field, `caps`. Build settings with `Setting::of` / `Setting::under`
  (`caps: None`) instead of a struct literal.
- `LimitsInForce` has a new field, `max_notional_usd`. In a struct literal, add
  `max_notional_usd: None`, or use `..` from an existing value. Its JSON is unchanged when the field
  is empty, and it reads JSON that leaves it out.

**Behaviour**
- Grid's hard stop also sends `StandDown` and a `grid_hard_stop` alert, which takes effect once the
  bot's host handles the intent.
