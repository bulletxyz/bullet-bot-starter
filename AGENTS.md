# bullet-bot-starter

Strategies for the Bullet bot (`bb`), built into WASM components (`wasm32-wasip2`) a bot runs as
plugins, and the docs for setting the bot up. No crate here depends on the bot.

## Build and test

```sh
just test            # native tests: SDK, examples, the built-ins and their parity replay
just build [name]    # dist/<name>.wasm + .sha256 (the folder name is the plugin's name)
just new <name>      # strategies/<name>/ from templates/strategy
just ci              # fmt, clippy -D warnings, tests, build
```

## Where things are

- `sdk/`: the guest SDK and the `bb:strategy` WIT world (`sdk/wit/`), the one source of both. The
  bot builds its WASM host against them and its tests from `strategies/ladder/`, at a pinned commit:
  keep the crate names, and a change that breaks a guest or the host is a new world version, never
  an edit of a released one.
- `support/`: small pure helpers the built-ins share (client ids, inventory tracker, IOC price cap, close price,
  funding math).
- `strategies/<name>/`: one crate each. `ladder` and `sma` are example plugins. `grid`, `rsi-momentum`,
  `simple-mm` (the market maker), `avellaneda-stoikov`, `funding-arb` and `simple-funding` are the bot's own
  built-ins: it imports them as git dependencies and runs them natively; they also build for `wasm32-wasip2`, but
  are not plugins yet. `parity/`: the recorded calls of the built-ins (grid, rsi-momentum) and their replay. Keep them green; a change that makes a
  fixture differ is a change in behaviour: say so in that strategy's `AGENTS.md`.
- `docs/`: [making](docs/MAKE-A-STRATEGY.md) and [installing](docs/INSTALL.md) a strategy, the
  [SDK cheat sheet](docs/SDK.md),
  [running the bot with Docker](docs/DEPLOY.md), the bot's [HTTP API](docs/API.md) and
  [sign requests and venue keys](docs/AUTH.md) (the docs a bot serves link these three: never move
  them). Setting a bot up is the guide Claude follows,
  `DEPLOY-WITH-CLAUDE.md` (the README's button links its raw URL on `main`: never move it).
- `skills/bullet-wasm-plugin/SKILL.md`: building and installing a plugin in Claude Code (the README's
  button links its raw URL on `main`). Keep it in step with the build. The bot links this path:
  never move it. `.github/workflows/ci.yml` also publishes a fork's plugins as a `plugins-<commit>`
  release on each push to its `main`; the guides don't send anyone there.

- A backup download page (an artifact) uses the `downloads` capability, never a `data:` link: see
  `docs/MAKE-A-STRATEGY.md`. The zip is attached in the chat first.

## A plugin

- Depends on `bb-strategy-sdk` (and `bb-strategy-support`) and pure crates only: no I/O, no clock
  (`View::now_ms`), no randomness, no `tracing`. Clippy denies panics and unchecked arithmetic.
- `Describe::NAME` is its folder name. Bump `version` in its `Cargo.toml` for any change.
- At most 64 intents a call; cancel only its own orders; never `Shutdown` (the bot faults it).
- A strategy only names its settings; the bot decides where they live: `[instances.<id>.plugins.<name>]` on the bot,
  changed live at `plugins.<name>.<setting>` on an instance. Any name is fine (`max_position` too): the bot never
  reads one as its own limit. Only `exchanges.<venue>.markets` for an exchange it declares is the bot's.

## Money safety

Money-safety code needs a failing test first: write the test that reproduces the case, see it fail, then
fix. That is all of `support/src/` (the bot re-exports `ioc`, `client_id`, `inventory` and `funding`; rsi-momentum
closes off `close_price`) and the files each built-in's own `AGENTS.md` lists.

## Writing for the user

When you give the user steps: numbered, one action per step, the action first; the fewest words;
caveats after, one line, or not at all; never open with what can't be done. Beside each list put a link to the exact page, filled in, only
if verified (docs or a fetch); with none, say there is none. They read as little as
they can: the one thing to do comes first.

## Safety (always)

- Paper first. Never set a strategy live for the owner: going live is the owner's signed change.
- The owner signs every plugin's hash before it runs. Show them the hash `just build` printed.
- Never ask for, accept, store or print a private key or seed phrase. The bot makes its own keys.
- Never commit keys, tokens, `.env` files or a bot's URL.
