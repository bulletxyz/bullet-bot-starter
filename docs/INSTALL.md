# Install a strategy on your bot

Update your bot first if it says a newer release is out (on Railway, redeploy the service in your [dashboard](https://railway.com/dashboard)).

## 1. Send it to the bot

Pick one:

- **Built in Claude Code:** Claude uploads it with a one-time link (`plugin_upload_link`). In a cloud
  session it first checks it can reach your bot and shows you how to allow its address, from [claude.ai/code](https://claude.ai/code). Every step:
  the [guide](../skills/bullet-wasm-plugin/SKILL.md).
- **File on your computer:** on your bot's page (Claude gives you its link), click *Add plugin* (at the bottom).
- **Plugin someone shared** at a public https link: tell Claude "Install the plugin at `<url>`,
  sha256 `<hash>`." (`install_plugin_from_url`)

Adding runs nothing.

## 2. Approve it

1. Check the hash on the approval card is the one your build printed.
2. Approve with your wallet (a sign link, only where no card shows). It signs:

```
approve plugin <name> v<version> sha256 <hash>
bot <bot id> request <request id>
```

A changed byte is a new hash and needs a new signature.

## 3. Run it on paper

Tell Claude "Run it on paper." It uses `plugin:<name>@<sha256>` and sets its settings (`describe`
lists them): real prices, no real orders. Bot already live? See the next section.

**A new version:** install and approve it the same way, then tell Claude "Move my instance to it."
(`apply_change` with `{instance, set: {strategy: "plugin:<name>@<new sha256>"}}`; settings carry over.)

## Already live? Run it next to it on paper

1. Tell Claude: "Run `<name>` on paper next to my live bot." It adds another instance on a free
   sub-account (`create_instance`) and your live one keeps going.
2. Approve the card, unless you turned on the `agent_manages_instances` unlock.
3. Give it a market your live instance doesn't trade, or you sign `account.same_symbol` = `allow`.

```json
{"id": "my-strategy-paper", "strategy": "plugin:my-strategy@<sha256>", "symbols": ["ETH-USD"],
 "params": {"max_position": "0.5", "daily_loss_usd": "20"},
 "plugins": {"my-strategy": {"spacing_bps": "10", "size": "0.01"}}}
```

- `params` are the instance's own limits (`max_position`, `daily_loss_usd`, `stale_secs`,
  `max_notional_usd`, `max_orders_per_min`, `dry_run`). The plugin's own settings go under
  `plugins.<name>` (`describe` lists them), even one also called `max_position`.
- It needs a sub-account of your pool (`sub_account` left out takes the first free one).
- Never switch a live instance to a plugin you haven't watched on paper.

## 4. Go live

Only after a paper run you like: tell Claude, then sign. The bot's limits, daily loss stop and kill
switch hold for plugins too.

To stop a version for good, tell Claude "Revoke it" (`revoke_plugin`): every instance running it
stops.

Keep every version you build (the `.wasm`, its source and its sha256):
[Keep every version safe](MAKE-A-STRATEGY.md#keep-every-version-safe).

Never send a private key or seed phrase to anyone, Claude included. A signature is safe to share.
