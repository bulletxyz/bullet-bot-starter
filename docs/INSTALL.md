# Install a strategy on your bot

Plugins built here need bot release v43 or later (the `bb:strategy@0.4.0` world). If your bot says a
newer release is out, update it first: on Railway, redeploy the service.

## 1. Send it to the bot

Pick the one that fits where the file is.

- **Built in a Claude chat's code sandbox:** Claude asks the bot for a one-time upload link
  (`plugin_upload_link`) and uploads the file to it. The
  [bullet-wasm-plugin guide](../skills/bullet-wasm-plugin/SKILL.md) has every step.
- **At a public https address** (a release on your fork, say): "Install the plugin at
  `<url>`, sha256 `<hash>`." (`install_plugin_from_url`)
- **On your computer:** on your bot's page, *Add plugin* (at the bottom). Or Claude sends a file it
  holds in parts (`install_plugin`).

The bot checks the file's sha256 and keeps it. Adding runs nothing.

## 2. Sign its hash

The bot sends you a sign link. Your wallet signs:

```
approve plugin <name> v<version> sha256 <hash>
bot <bot id> request <request id>
```

Check the hash is the one your build printed. A changed byte is a new hash and needs a new
signature.

## 3. Run it on paper

Ask Claude to switch to it. The strategy is `plugin:<name>@<sha256>`, and it starts on paper:
real prices, no real orders. Claude sets its settings the same way (`describe` lists them).

**A new version** of a plugin you already run: install and sign it the same way, then ask Claude
to move the instance to it (`apply_change` with `{instance, set: {strategy: "plugin:<name>@<new
sha256>"}}`). Its settings carry over.

## 4. Go live

Only after a paper run you like. Going live is your signed change, as for any strategy. The bot's
limits, daily loss stop and kill switch hold for plugins too.

To stop a version for good, ask Claude to revoke it (`revoke_plugin`): every instance running it
stops.

Keep every version you build (the `.wasm`, its source and its sha256):
[Keep every version safe](MAKE-A-STRATEGY.md#keep-every-version-safe).

Never send a private key or seed phrase to anyone, Claude included. A signature is safe to share.
