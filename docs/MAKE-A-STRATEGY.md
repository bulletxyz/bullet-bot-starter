# Make your own strategy

A strategy is a small Rust crate: the bot shows it the market, it answers with orders. It never
holds a key, reads a clock or touches the network. The bot checks every order against your limits.

Agents: clone the repo first (`git clone https://github.com/<owner>/bullet-bot-starter`, the
user's copy if they have one, else this one) and read the [SDK cheat sheet](SDK.md), the template and
the examples from the checkout, not file by file over the web.

## You need

- [Rust](https://rustup.rs) through `rustup`: in this folder, `rustup toolchain install` installs the
  version `rust-toolchain.toml` pins, with the `wasm32-wasip2` target.
- [just](https://github.com/casey/just): `cargo install just`.

## Make one

```sh
just new my-strategy
```

That makes `strategies/my-strategy/` from a template that already works: one buy under the mid and
one sell over it, moved when the mid moves a spacing, within a largest position, with partial fills
and unknown outcomes handled. The folder name is the strategy's name (`Describe::NAME`): the bot
shows it and your wallet signs it, and the build refuses a file labelled otherwise.

Edit `strategies/my-strategy/src/lib.rs`:

- `on_event` gets each event (`Book`, `Fill`, `Tick`, an order's `Outcome`) with a `View`: the book,
  your position, P&L, the time (`now_ms`). It returns intents: `Place`, `Cancel`, `CancelAll`.
  A `Place` can carry `protect`: a stop (and take-profit) the venue holds for the position once it
  fills.
- What every event, view field and intent holds: the [SDK cheat sheet](SDK.md).
- `stop` runs when the bot winds down: return `CancelAll`.
- `report` is what the bot's page shows about it.
- `Settings` and `describe()` are the settings the owner can change, with ranges.
  Name them as you like: on the bot each instance keeps its own table of them, `[instances.<id>.plugins.<name>]`,
  changed live at `plugins.<name>.<setting>`, and the bot never takes one for its own limits. Only
  `exchanges.<venue>.markets` for an exchange you declare is the bot's.

More to copy from: [ladder](../strategies/ladder/) (retries, unknown outcomes, venue reads).
[sma](../strategies/sma/) warms up on past candles: the bot gives a plugin up to 999 bars at start.

## Test and build

```sh
just test                 # native tests (the SDK's SimHost runs a strategy like the bot does)
just build my-strategy    # dist/my-strategy.wasm and dist/my-strategy.wasm.sha256
```

The same source and toolchain give the same file in any folder, so the same hash. Bump `version`
in its `Cargo.toml` for every change.

Claude Code can do all of it and upload the plugin to your bot: the
[bullet-wasm-plugin guide](../skills/bullet-wasm-plugin/SKILL.md).

## Keep every version safe

A plugin you lose is a plugin you can't run, check or roll back to. Keep its source and each
`.wasm` with its sha256 wherever you keep your code. Bump `version` for each change, so the owner's
approval names it.

**A download page.** If the user wants one, attach the zip (every source file, each `.wasm`, its
sha256) in the chat first: it is their backup if the page fails. Then publish an artifact page that
embeds the zip as base64 data and declares the `downloads` capability (`capabilities: {downloads:
true}`). The button saves with `(await claude.use("downloads")).save({filename, data: blob})` and is
hidden when `use()` returns `null`. A `data:` link or `<a download>` does nothing in the artifact
viewer.

## Rules the bot enforces

- At most 64 intents a call. Prices above 0 for resting orders.
- Cancel only your own orders. Your `CancelAll` takes only yours, never the owner's.
- No `Shutdown`: a plugin can't stop the bot (it faults). To stop, cancel and send nothing more.
- Time and data come only from the bot: no files, network, clock or randomness.
- Fuel and memory limits per call: a strategy that runs away is stopped and its orders cancelled.

Next: [Install it on your bot](INSTALL.md).
