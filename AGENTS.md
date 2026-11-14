# bullet-bot-starter

Strategies for the Bullet bot (`bb`), built into WASM components (`wasm32-wasip2`) a bot runs as
plugins, and the docs for setting the bot up. No crate here depends on the bot.

## Build and test

```sh
just test            # native tests: SDK, examples, the ports' parity with the bot's built-ins
just build [name]    # dist/<name>.wasm + .sha256 (the folder name is the plugin's name)
just new <name>      # strategies/<name>/ from templates/strategy
just ci              # fmt, clippy -D warnings, tests, build
```

## Where things are

- `sdk/`: the guest SDK and the `bb:strategy` WIT world (`sdk/wit/`), the one source of both. The
  bot builds its WASM host against them and its tests from `strategies/ladder/`, at a pinned commit:
  keep the crate names, and a change that breaks a guest or the host is a new world version, never
  an edit of a released one.
- `support/`: small helpers copied from the bot (client ids, the IOC price cap, the close price).
- `strategies/<name>/`: one plugin each. `parity/`: the built-ins' recorded calls and their replay.
- `docs/`: [making](docs/MAKE-A-STRATEGY.md) and [installing](docs/INSTALL.md) a strategy,
  [running the bot with Docker](docs/DEPLOY.md), the bot's [HTTP API](docs/API.md) and
  [sign requests and venue keys](docs/AUTH.md) (the docs a bot serves link these three: never move
  them). Setting a bot up is the guide Claude follows,
  `DEPLOY-WITH-CLAUDE.md` (the README's button links its raw URL on `main`: never move it).
- `skills/bullet-wasm-plugin/SKILL.md`: building and installing a plugin from a Claude chat. Keep it
  in step with the build. The bot links this path: never move it.

## A strategy

- Depends on `bb-strategy-sdk` (and `bb-strategy-support`) and pure crates only: no I/O, no clock
  (`View::now_ms`), no randomness, no `tracing`. Clippy denies panics and unchecked arithmetic.
- `Describe::NAME` is its folder name. Bump `version` in its `Cargo.toml` for any change.
- At most 64 intents a call; cancel only its own orders; never `Shutdown` (the bot faults it).
- Settings it declares must not be named `symbol`, `dry_run`, `state_dir`, `max_position`, `daily_loss_usd`,
  `stale_secs`, `max_notional_usd` or `max_orders_per_min` (nor one under them), nor `legs.<leg>.symbols`: the bot
  keeps those for itself and refuses such a file when it is added, before anyone signs.
- A change to a port's logic needs its parity tests green; a change that makes a fixture differ is
  a change from the built-in: say so in the port's `adapter.rs` docs.

## Safety (always)

- Paper first. Never set a strategy live for the owner: going live is the owner's signed change.
- The owner signs every plugin's hash before it runs. Show them the hash `just build` printed.
- Never ask for, accept, store or print a private key or seed phrase. The bot makes its own keys.
- Keep every built plugin: after each `just build`, keep a Claude artifact page listing each version and
  its sha256, with the `.wasm` (or a zip with its source) embedded in the page as a download (artifacts
  can't store a `.wasm` as a file), republished for each version; in a fork of
  https://github.com/bulletxyz/bullet-bot-starter, commit each version naming its sha256.
- Never commit keys, tokens, `.env` files or a bot's URL.
