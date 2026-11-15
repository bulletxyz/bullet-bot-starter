---
name: bullet-wasm-plugin
description: Build and install a Bullet bot strategy plugin (a WASM file) from a Claude chat with a code sandbox. Use when the user wants a strategy the bot doesn't have, or to build, back up, upload or run a plugin from the bullet-bot-starter repo.
---

# Build and install a Bullet strategy plugin

The plugin is a Rust crate built to `wasm32-wasip2`. You build it in the chat's code sandbox,
send the file straight to the bot, and the owner signs its hash. Nothing runs until they do.

Never ask for, accept or print a private key, seed phrase or token. The bot makes its own keys.
Never take a plugin live: going live is the owner's signed change. Paper first.

The bot must be connected to this chat (its tools listed) and run release v43 or later. If its tools
are missing, the owner connects it first from the bot's page (*Connect to Claude*). Its `plugins` tool
says what to allow and links this guide.

Work in a copy of https://github.com/bulletxyz/bullet-bot-starter (`git clone`). Read
`docs/MAKE-A-STRATEGY.md` for the strategy API. Tell the user each step in a line, not a lecture.

## 1. Allow the network (the owner does this once)

Settings > Capabilities > Code execution, on the web or desktop app. The phone app doesn't show
the option, so ask them to open it there. Allow:

- package managers,
- `static.rust-lang.org` (the Rust installer, toolchain and WASM target, which no package manager has),
- `github.com` (the `git clone`),
- the bot's domain (for the upload in step 6).

If a download is blocked, say which domain and ask them to add it.

## 2. Install the toolchain

`just` is not in the sandbox: use the plain commands below, which are what the justfile runs.

```sh
curl --proto '=https' --tlsv1.2 -sSf https://static.rust-lang.org/rustup/rustup-init.sh | sh -s -- -y --profile minimal
. "$HOME/.cargo/env"
git clone https://github.com/bulletxyz/bullet-bot-starter
cd bullet-bot-starter
rustup toolchain install    # the toolchain rust-toolchain.toml pins, with wasm32-wasip2, rustfmt, clippy
```

## 3. Write, test, lint

Make a new one with `scripts/new-strategy.sh <name>` (what `just new` runs) and copy what you need
from `strategies/ladder/`. The folder name is the plugin's name; `Describe::NAME` must match it.
Bump `version` in its `Cargo.toml` for every change.

```sh
scripts/new-strategy.sh my-strategy
cargo fmt --all
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

The lints are strict (every warning fails) and flag small things. Expect:

- `unwrap`, `expect`, `panic!`, `todo!`, `dbg!`: use `?`, `let ... else`, `.get()`, `.ok_or(...)`.
- `+ - * /` on numbers (`arithmetic_side_effects`): use `checked_add`, `checked_mul`,
  `saturating_sub`; on `Decimal`, `checked_*` and handle `None`.
- an `#[allow]` without a `reason = "..."` (use `#[expect(lint, reason = "...")]`).
- pedantic style: `as` casts (use `try_from` or `From`), `needless_pass_by_value`, missing
  backticks in doc comments, `doc_markdown`, `too_many_lines`, `cast_possible_truncation`,
  `float_cmp`, redundant clones, `unused_qualifications`.

No I/O, clock, randomness or `tracing` in a strategy: use `View::now_ms` and the SDK. Fix every
warning; don't silence one unless it is true and you give the reason.

## 4. Build the .wasm and record its hash

```sh
scripts/build.sh my-strategy      # dist/my-strategy.wasm, .sha256, and a table of name, version, hash
```

The build refuses a file whose name or version label (what the bot reads before the owner signs)
is not the folder's name and the `Cargo.toml` version. Write the hash down.

## 5. Back it up before anything else

Zip the source, the `.wasm` and the sha256 and attach the zip in the chat. Then keep a plugins
page: a Claude artifact listing each version (version, sha256, date, notes). Artifacts can't host a
`.wasm` or `.zip` as a file, so embed the zip in the page as a base64 `data:` download button
(pages are capped at 16 MB). Republish the page for each new version.

A technical user instead commits each version in their own fork (message names the version and
sha256, plus a tag).

## 6. Upload with the one-time link

The file goes from the sandbox straight to the bot, never through the chat.

1. Call the bot's MCP tool `plugin_upload_link` with `{name, version, size, sha256}` (`size` is
   the file's byte count as an integer; `wc -c < dist/my-strategy.wasm`). It returns `{url}`,
   good for 10 minutes and one upload.
2. `curl --upload-file dist/my-strategy.wasm '<url>'` (an HTTP PUT; quote the url).
3. The answer is JSON with the sha256 the bot computed and the owner's sign link (`sign.link`).
   Check that sha256 equals the one you recorded. If not, stop: don't send the link; rebuild and
   retry.
4. Send the owner the sign link. They sign `approve plugin <name> v<version> sha256 <hash>` in
   their wallet; they should see the same hash. Don't go on until they have.

If `plugin_upload_link` is missing, the client's tool list is stale: refresh the connector or
start a new chat. If the sandbox can't reach the bot, step 1's domain isn't allowed yet. If the bot
refuses the link (it has no public address), send the file with `install_plugin` instead: in parts
of at most 700 KiB of file each, base64. Or the owner downloads the zip from step 5 and adds the
`.wasm` on the bot's page (*Add plugin*, at the bottom).

## 7. Start it on paper

Find it with the `plugins` tool, then switch with `propose_change` and `apply_change`: the
strategy is `plugin:<name>@<sha256>`. Check what it needs with `describe` or `strategies` first.
Settings must have the right types or the instance crashes:

- numbers as numbers (`10`, `0.5`), not strings;
- booleans as `true` / `false`, never the string `"true"`;
- the market in a list (`["BTC-USD"]`), not a bare string.

A new version of a plugin an instance already runs: after the owner signs it, move the instance
with `apply_change` `{instance, set: {strategy: "plugin:<name>@<new sha256>"}}`. Its settings
carry over.

Stay on paper. Read `paper_report` after a while and tell the owner how it did; going live is only
ever their signed change, and they choose it.
