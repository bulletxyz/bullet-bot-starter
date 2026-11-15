# Make your own strategy

A strategy is a small Rust crate: the bot shows it the market, it answers with orders. It never
holds a key, reads a clock or touches the network. The bot checks every order against your limits.

## You need

- [Rust](https://rustup.rs) through `rustup`: in this folder, `rustup toolchain install` installs the
  version `rust-toolchain.toml` pins, with the `wasm32-wasip2` target.
- [just](https://github.com/casey/just): `cargo install just`.

## Make one

```sh
just new my-strategy
```

That makes `strategies/my-strategy/` from a template that already works: one buy under the mid and
one sell over it, placed again when filled. The folder name is the strategy's name (`Describe::NAME`):
the bot shows it and your wallet signs it, and the build refuses a file labelled otherwise.

Edit `strategies/my-strategy/src/lib.rs`:

- `on_event` gets each event (`Book`, `Fill`, `Tick`, an order's `Outcome`) with a `View`: the book,
  your position, P&L, the time (`now_ms`). It returns intents: `Place`, `Cancel`, `CancelAll`.
  A `Place` can carry `protect`: a stop (and take-profit) the venue holds for the position once it
  fills.
- `stop` runs when the bot winds down: return `CancelAll`.
- `report` is what the bot's page shows about it.
- `Settings` and `describe()` are the settings the owner can change, with ranges.
  Name them as you like: on the bot each instance keeps its own table of them, `[instances.<id>.plugins.<name>]`,
  changed live at `plugins.<name>.<setting>`, and the bot never takes one for its own limits. Only
  `legs.<leg>.symbols` for a leg you declare is the bot's.

More to copy from: [ladder](../strategies/ladder/) (retries, unknown outcomes, venue reads),
[grid-wasm](../strategies/grid-wasm/) and [rsi-momentum-wasm](../strategies/rsi-momentum-wasm/).
[sma](../strategies/sma/) warms up on past candles: the bot gives a plugin up to 999 bars at start.

## Test and build

```sh
just test                 # native tests (the SDK's SimHost runs a strategy like the bot does)
just build my-strategy    # dist/my-strategy.wasm and dist/my-strategy.wasm.sha256
```

The same source and toolchain give the same file in any folder, so the same hash. Bump `version`
in its `Cargo.toml` for every change.

A Claude chat with a code sandbox can follow the [bullet-wasm-plugin skill](../skills/bullet-wasm-plugin/SKILL.md):
the toolchain, build, backup and upload in order.

## Keep every version safe

A plugin you lose is a plugin you can't run, check or roll back to. After each `just build`:

- **Most people:** ask Claude to keep it: Claude keeps a page (an artifact) listing each version and its
  sha256, with the `.wasm` (or a zip of source, `.wasm` and sha256) built into the page as a download,
  and publishes the page again for each new version. Artifacts can't store a `.wasm` as a file of its own.
- **Technical users:** work in your own fork of
  [bullet-bot-starter](https://github.com/bulletxyz/bullet-bot-starter) and commit each version, with
  a tag or commit message naming its sha256.

## Rules the bot enforces

- At most 64 intents a call. Prices above 0 for resting orders.
- Cancel only your own orders. Your `CancelAll` takes only yours, never the owner's.
- No `Shutdown`: a plugin can't stop the bot (it faults). To stop, cancel and send nothing more.
- Time and data come only from the bot: no files, network, clock or randomness.
- Fuel and memory limits per call: a strategy that runs away is stopped and its orders cancelled.

Next: [Install it on your bot](INSTALL.md).
