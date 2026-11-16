---
name: bullet-wasm-plugin
description: Build and install a Bullet bot strategy plugin (a WASM file) in Claude Code. Claude writes it from the bullet-bot-starter template, tests and builds it, and uploads it to the bot, where the owner approves it. Use when the user wants a strategy the bot doesn't have, or to build, update or install a plugin.
---

# Build and install a Bullet strategy plugin

A plugin is a small Rust crate built to `wasm32-wasip2`. You build it here in Claude Code (on the
web, in the desktop app's Code tab or in a terminal), upload it to the bot, and the owner approves
its fingerprint (its sha256). Nothing runs until they do, and it starts on paper.

Never ask for, accept or print a private key, seed phrase or token. Never take a plugin live: going
live is the owner's signed change.

**When you give the user steps:** numbered, one action per step, the action first; the fewest words;
caveats after, one line, or not at all; never open with what can't be done. Put a link straight to
the page beside each step list, filled in yourself (the bot's page is `https://<host>/`): never a
placeholder for them to fill. Where no link exists, say so in one line.

## 1. Get the code and the toolchain

Clone the starter and read `docs/SDK.md` (every type on one page), the template and the examples
from the checkout, not file by file over the web:

```sh
git clone https://github.com/bulletxyz/bullet-bot-starter   # or the user's own copy, if they have one
cd bullet-bot-starter
rustup toolchain install    # the Rust rust-toolchain.toml pins, with wasm32-wasip2, rustfmt, clippy
```

No `rustup`? Install it from https://rustup.rs. No `just`? The justfile shows the plain commands.

## 2. Pick the strategy and write it

If the user hasn't said what it should do, ask, in their words. Unless they want something else,
start from the template: one buy under the mid and one sell over it, moved with the mid, within a
largest position.

```sh
just new my-strategy      # strategies/my-strategy/ from templates/strategy
```

The folder name is the plugin's name: `Describe::NAME` must be the same. Bump `version` in its
`Cargo.toml` for every change. No I/O, clock, randomness or `tracing`: time is `View::now_ms`. Never
return `Reply`, `Persist`, `ClosePosition`, `SetLimits`, `StandDown` or `Targets`: they are the bot's built-ins' only,
and a plugin that returns one faults (`CancelOwn` crosses as `CancelAll`, `Notify` is dropped). More to copy from:
`strategies/ladder/` (retries, unknown outcomes, venue reads) and `sma` (past candles; up to 999 bars at
start).

Need `bb-strategy-support` (`capped_ioc_price`, `close_reference`, `ClientIdIssuer::own`) or another
crate? Add it to the strategy's `Cargo.toml`, then run `cargo update --workspace --offline` before
any `--locked` command (`just test` and `just lint` use `--locked`, and fail until the lock has it).

## 3. Test and lint

```sh
cargo fmt --all
just test
just lint
```

The lints are strict: every warning fails. Expect:

- `unwrap`, `expect`, `panic!`, `todo!`, `dbg!`: use `?`, `let ... else`, `.get()`, `.ok_or(...)`
  (tests may unwrap).
- `+ - * /` on numbers (`arithmetic_side_effects`): use `checked_add`, `checked_mul`,
  `saturating_sub`; on `Decimal`, `checked_*` and handle `None`.
- an `#[allow]` without a reason: use `#[expect(lint, reason = "...")]`.
- pedantic style: `as` casts (use `try_from` or `From`), `needless_pass_by_value`, missing
  backticks in doc comments, `too_many_lines`, `float_cmp`, identical match arms, redundant clones.

## 4. Build it

```sh
just build my-strategy    # dist/my-strategy.wasm, its .sha256, and a table of name, version, hash
```

The build refuses a file whose name or version label is not the folder's name and the
`Cargo.toml` version. Note the hash.

## 5. Upload it to the bot

First check this session can reach the bot. A session on the user's own computer (the desktop app's
Code tab on "Local", or a terminal) needs nothing. A cloud session (claude.ai/code, the mobile app's
Code tab, or the desktop app with a Cloud environment) only reaches the sites its environment's Network access allows:

**Find the bot's address.** Call the bot's `setup` tool and take the host of `connect.mcp_url` (for
example `mybot.up.railway.app`). No bot tools in this session? Ask the user once to paste their
bot's link (the address of their bot's page) and take its host.

**Test it.** `curl -sS -o /dev/null -w '%{http_code}' https://<host>/health`. Any normal HTTP status
means it is reachable: carry on and say nothing about it. Blocked looks like HTTP 403 with the
header `x-deny-reason: host_not_allowed`, or a DNS or connection error. On their own computer nothing
is blocked, so this passes and you skip the rest.

**If blocked,** send the user these steps with their host filled in.

In the Claude app or at [claude.ai/code](https://claude.ai/code) (that link opens Claude Code; the
setting itself has no direct link):

1. Click the environment selector (the first button under the message box).
2. Under Cloud, hover your environment and click the settings icon on its right.
3. Click Network access. If it says Full, you're done: nothing to add. If it says None, choose
   Limited.
4. With Limited selected, add `<host>` on its own line under Allowed domains (keep "Allow package
   managers" ticked).
5. Click Save changes, then tell me "done".

No settings icon in the app? Do the same steps at [claude.ai/code](https://claude.ai/code) in a
browser. Access levels: [Claude Code docs](https://code.claude.com/docs/en/cloud-environments#network-access).

It reaches this session within a minute. Shared by an organization? An admin changes it in its
settings (Cloud environments).

**Test again, then upload:**

1. Call the bot's `plugin_upload_link` with `{name, version, size, sha256}` (`size` is the file's
   byte count, `wc -c < dist/my-strategy.wasm`). It returns `{url}`, good for 10 minutes and one
   upload.
2. `curl --upload-file dist/my-strategy.wasm '<url>'` (quote the url).
3. The answer has the sha256 the bot computed: check it equals yours. If not, stop and rebuild.
4. Call `install_plugin` with that sha256: the owner approves on its card in the chat (send them its
   `sign.link` only where no card shows; the upload's answer has none, so a plugin gets one sign
   request). Their wallet signs `approve plugin <name> v<version> sha256 <hash>`; they
   should see the same hash. Don't go on until they have.

**No bot tools in this session?** Send the user one of these (the first that fits):

1. In a terminal or the desktop app: run `claude mcp add --transport http bullet-bot https://<bot domain>/mcp`, then
   sign in with `/mcp` ([docs](https://code.claude.com/docs/en/mcp)).
2. Send this in your bot's Claude chat: "Give me a plugin upload link for `<name>` v`<version>`, size
   `<bytes>`, sha256 `<hash>`", and paste the link here. You then upload to it.
3. Nothing reaches the bot? On [your bot's page](https://<host>/), click *Add plugin* (at the bottom) and pick the `.wasm`.

A plugin someone published at a public link (a shared, ready-made strategy) installs with
`install_plugin_from_url` and its sha256 instead.

## 6. Start it on paper

Find it with the `plugins` tool, then switch with `propose_change` and `apply_change`: the
strategy is `plugin:<name>@<sha256>`. Check what it needs with `describe` or `strategies` first.
Settings must have the right types or the instance crashes:

- numbers as numbers (`10`, `0.5`), not strings;
- booleans as `true` / `false`, never the string `"true"`;
- the market in a list (`["BTC-USD"]`), not a bare string.

Stay on paper. Read `paper_report` after a while and tell the owner how it did. Going live is only
ever their signed change, and they choose it.

### Already live? Run it next to it on paper

If the bot's instances are live, add the plugin as another instance on a free
sub-account (`create_instance`, paper, the live one untouched):

```json
{"id": "<name>-paper", "strategy": "plugin:<name>@<sha256>", "symbols": ["ETH-USD"],
 "params": {"max_position": "0.5", "daily_loss_usd": "20"},
 "plugins": {"<name>": {"<setting>": 1}}}
```

- `symbols` is the market. Choose one no other instance trades, or the owner signs
  `account.same_symbol` = `allow` (`propose_change`).
- `params` are the instance's own limits (`max_position`, `daily_loss_usd`, `stale_secs`,
  `max_notional_usd`, `max_orders_per_min`, `dry_run`). The plugin's own settings, whatever they are
  called, go under `plugins.<name>` (typed, as above); a plugin's `max_position` there is its own.
- Needs the owner's signature (send them the card; the `sign.link` only where no card shows) or
  their `agent_manages_instances` unlock, and a free sub-account of their pool (omit `sub_account`
  for the first free one).

Never switch a live instance to a plugin that hasn't run on paper: it takes the live strategy off the market.

## A new version

Change it, bump `version`, test, build and upload it the same way. After the owner approves it,
move the instance with `apply_change` `{instance, set: {strategy: "plugin:<name>@<new sha256>"}}`;
its settings carry over.

Ask the user where they want to keep the source and each `.wasm` with its sha256: a plugin they
lose can't be checked or rolled back to.

A download page is optional: attach the zip in the chat first, then publish an artifact with the
`downloads` capability (`capabilities: {downloads: true}`) that embeds it and saves with
`(await claude.use("downloads")).save({filename, data: blob})`, hiding the button when `use()` returns
`null`. Never a `data:` link: the viewer blocks it. Details: `docs/MAKE-A-STRATEGY.md`.
