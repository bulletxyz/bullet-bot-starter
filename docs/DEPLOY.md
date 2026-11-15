# Run it with Docker

[Deploy with Claude](../README.md#1-get-your-bot) puts your bot on Railway for you. To run it on your
own server instead, use the bot's public image, `ghcr.io/bulletxyz/bullet-bots` (linux/amd64 and
linux/arm64). `:latest` follows the newest release; pin a release tag such as `:v<N>` where an
unplanned upgrade matters.

```sh
docker run -d --name bb -p 3030:3030 -v bb-data:/data \
  -e BB_ALLOWED_WALLETS=<your Solana wallet> \
  ghcr.io/bulletxyz/bullet-bots:latest
docker logs bb   # the bot's address and what to do next
```

Keep the `-v`: the volume holds the bot's trading key, its history and every setting. Without it
they are lost with the container.

The first start makes the bot's trading key on the volume and prints only its address. It runs the
market maker on Bullet mainnet, **on paper**. Until you add the bot's address as a delegate in
[Bullet's app](https://app.bullet.xyz) (*Delegate accounts*), it can only paper trade; it picks the
delegation up without a restart. A delegate can trade your account and never withdraw.

Run it on a server outside the US: the bot's price feeds refuse US servers. Railway's Singapore
region is where Deploy with Claude puts it.

## Reach it from Claude

Claude, and every sign link the bot sends you, need the bot on a public https address:

1. Put it behind https on a hostname you control (a reverse proxy such as Caddy) and set
   `BB_PUBLIC_HOST=<that hostname>`. Have the proxy overwrite one header with the caller's address
   and name it in `BB_TRUSTED_CLIENT_IP_HEADER` (Caddy: `header_up X-Real-IP {remote_host}`, then
   `X-Real-IP`). The proxy must be the only way to the bot.
2. In Claude: *Settings > Connectors > Add custom connector*, paste `https://<hostname>/mcp`,
   *Connect*, and sign with your wallet.

Never publish the port over plain http on a public address.

## Variables

- `BB_ALLOWED_WALLETS`: the Solana wallet(s) that own the bot, comma separated. Without one the bot
  is unowned until claimed through the one-time link it logs.
- `BB_PUBLIC_HOST`: the https hostname the bot is reached at. On Railway the bot takes it from the
  service's domain.
- `BB_TRUSTED_CLIENT_IP_HEADER`: behind your own proxy, the header it overwrites with the caller's
  address. Never `X-Forwarded-For`.
- `BB_MM_DRY_RUN`: `true` forces paper trading, whatever the config says.
- `BB_MM_MAX_POSITION`, `BB_MM_DAILY_LOSS_USD`: hard limits no change goes above.
- `PORT`: the port to listen on (default 3030).
