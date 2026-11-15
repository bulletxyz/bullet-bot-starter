# Sign requests and venue keys

What your wallet signs for your bot, where the link to sign it goes, and how the bot gets its own
trading key without ever holding yours.

## Sign requests

Every owner signature the bot asks for (a device login, a config change, a rollback, an
unlock) is a **sign request** with an id: `{id, kind, summary, message, expires_ms, state}`,
`state` one of `pending`, `done`, `expired`, `failed`. The id (128 random bits) is the
capability: whoever holds it can read the message and post a signature, and the bot takes
only an allowed wallet's signature over the stored message, before its expiry, once. Then it
does what was signed and keeps the result readable for 10 minutes past the expiry.

- `GET /api/v1/sign/{id}`: the request as below, with its `state`, `result?`, `error?` (no
  auth; nothing in it is secret).
- `POST /api/v1/sign/{id}` `{wallet?, signature}`: submit the signature (`wallet` names the
  signer; without it every allowed wallet is tried). 404 unknown, 410 expired or already
  used, 403 not a good signature, 409 the action failed.
- A signed request reads `applying` while the bot performs it, a config change or a one-tap venue
  request alike: the page and `bb sign wait` keep waiting, and neither offers it to sign again.
- A signed fund or delegation that isn't confirmed blocks that step: the bot opens no other request
  for it and sends no other signature, and says it may have applied and to check your balance
  first. That clears once Bullet shows the whole move, or about 20 minutes after the request was
  opened. Only Bullet's own refusal, or a send that never left, clears it at once: a lost reply may
  follow a move that landed. The setup page and `/ui/api/tx/submit` go through the same block.
- `POST /api/v1/sign/{id}/opened`: the bot's own page pings this when it loads, and `opened` turns
  true. No auth and no body; it changes nothing else and answers nothing but 204 (404 unknown).
- A `needs_owner_signature` answer (and a device login's start, a one-tap venue
  request) carries the request as `sign`: `{id, link, public, kind, action, summary, message,
  expires_ms, bot, context?, opened}`. `link` is the bot's own page or `null` (below); `message` is
  exactly what the wallet signs (a Bullet transaction: its exact message; a `bullet_deposit`, the deposit
  of the owner's Solana USDC into their own Bullet account: the Solana transaction's message in
  base64, signed with `signTransaction` on the bot's own page only, since its blockhash lasts
  about a minute); `bot` is the bot's origin (informational); `context` is a venue request's
  public facts the page checks (the bot's address, its sub-account, the network, the owner).

### Which link

- **A caller on the bot's machine** (the CLI, the TUI, `bb setup`'s Claude login, `bb call`,
  Claude Code talking to `127.0.0.1`) gets the bot's own page on the address it reached:
  `http://127.0.0.1:<port>/sign#id=<id>`: short enough for a QR code, and the page submits the
  signature itself. A caller counts as on this machine only when its TCP connection is loopback
  at both ends, its `Host` is `localhost`, `127.x` or `[::1]`, and the request carries no proxy
  header (`X-Forwarded-For`, `Forwarded`, `X-Real-IP`, ...): a proxy on the same machine
  connects over loopback, so a request through it links like anyone else's.
- **A public bot** (`BB_PUBLIC_HOST` set, on Railway, or any non-loopback host; `public: true`)
  links to its own page: `<bot>/sign#id=<id>`. The page reads the request, the wallet signs,
  the page posts it.
- **Any other caller of any other bot** (on `localhost`, behind NAT) gets no link (`link:
  null`, `public: false`): the owner's browser can't reach the bot, so nothing can be
  signed until it can. Give the bot a public address (`BB_PUBLIC_HOST`), or open its page from a
  browser on the bot's machine.

The page is the bot's own `/sign`, served by the bot itself.

### Security

- The bot only accepts a signature over the exact message it stored. A config change's
  message is bound to the bot (`<network>:<trading key address>`), the config version in
  force, a fresh nonce and an expiry, and the safety gate checks all four again; each
  request is single use. A page that altered the message would get a signature the bot
  refuses.
- Your wallet shows what you sign: a login, change, rollback or unlock is a text message
  (`signMessage`); a one-tap venue request (delegating, funding, a deposit) is that venue's own
  transaction, exactly as the bot stored it.
- A request always comes from the bot: the page reads it from `/api/v1/sign/{id}`, and anything
  else in a link is ignored.

## Venue keys by address

The bot makes its own trading key on each venue and shows only its public address. You
authorize that address in the venue's own app with your own wallet: paste it in **Bullet >
Delegate accounts**, or in **Hyperliquid > API** ("API wallets"). The bot never relays your
signature; it counts a Hyperliquid key approved only once Hyperliquid lists it for your
account (`userRole` / `extraAgents`). An agent key places orders but cannot withdraw.

