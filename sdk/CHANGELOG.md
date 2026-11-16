# bb-strategy-sdk changelog

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
