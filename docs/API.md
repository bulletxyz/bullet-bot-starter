# The HTTP API

Everything here is served by the bot itself, on the same port as `/health`, and needs the
**owner**: a device token (`bb auth login`) as `Authorization: Bearer`, or
the wallet session the signing page (`/sign`) opens, which the browser holds as an
`HttpOnly`, `SameSite=Strict` cookie. A bot on `localhost` uses the cookie `bb_owner`
over plain http (Safari refuses `Secure` cookies there); a bot on a public host uses
`__Host-bb_owner` with `Secure` and ignores a plain one (a sibling subdomain can set
those). An OAuth access token (what Claude gets for `/mcp`) is **not** an owner session
and is refused here. A device token is let in but is authorized as the agent: its changes
are recorded `by: agent`, and releasing the kill switch, phone alerts (`/api/v1/alerts`) and
`/ui/api/connections/revoke-all` answer it `403` `{"kind": "needs_owner", "link"}`: the
owner does those on the bot's page, signed in with their wallet.

| Endpoint | Returns |
|---|---|
| `GET /api/v1/session` | `{wallet, network}` of the signed-in owner (401 when signed out) |
| `GET /api/v1/overview` | the account (halt, limits, totals) and per instance: full snapshot, market, pending delegation |
| `GET /api/v1/market` | per instance: best bid/ask, mid, fair value and source, basis, our quotes |
| `GET /api/v1/fills?instance&from&to&limit` | fills in `[from, to]` ms (default the last day), newest `limit` (max 1000) |
| `GET /api/v1/series?kind&instance&from&to` | `[ms, value]` points; `kind` is `pnl`, `inventory`, `fill_rate`, `spread_captured`, `markout_5s`, `markout_60s` |
| `GET /api/v1/journal?instance&n` | parameter changes and incidents, oldest first |
| `GET /api/v1/logs?instance&level&since_ms&limit` | the bot's recent log lines |
| `GET /api/v1/connections` | the MCP address and the apps signed in to it; `POST /ui/api/connections/revoke-all` signs them all out |
| `GET /api/v1/checklist` | the steps from paper to live, per instance, with the delegate link and the connect command |
| `GET /api/v1/paper-report` | per instance: the paper report (`verdict` `collecting`, `not_yet` with `reasons`, `ready` with `try_live_usd` and the `limits` a go-live keeps; fills, P&L, worst drawdown, hours quoting, picked-off share) and its `card` (none when live) |
| `GET /api/v1/staged` | the staged paper twin and its comparison with the running bot (`staged: null` when none), the promotion body and link |
| `GET /api/v1/export?what&format&kind&instance&from&to` | `what` is `fills`, `series` or `staged`; `format` is `csv` (default) or `json` |
| `GET /api/v1/catalog?kind&name&q&page&per_page` | every strategy and exchange with every setting (path, type, range, tier, help, default, current value), paged over settings (`per_page` default 200, max 1000); `kind` is `strategy` or `exchange`, `q` finds text in a path or help. Returns `{strategy, strategies, exchanges, page, per_page, pages, total}`; each strategy has `in_force`, `missing` and `needs` (venues, keys, reference prices), each exchange `configured` and `inputs` |
| `GET /api/v1/catalog/<kind>/<name>` | one strategy or exchange with all its settings and current values (404 when unknown) |
| `GET /api/v1/onboarding?strategy=<name>&set.<path>=<value>...` | the steps to switch to a strategy, in order, and the first one still open (`next`): choose its exchanges, connect each non-Bullet exchange's bot-held key, give the required settings that have no default, review and sign. `set.` pairs lay the change being composed (config paths) over the stored values. Each step is `{id, kind, venue?, role?, setting?, done, detail, key_status?, bot_address?, choices?, action?}`; the same steps are on each `/api/v1/catalog` strategy row as `onboarding` (with its `descriptor`), and over MCP as `onboarding`. Owner session only; a key shows as its state and public address, never a key |
| `GET /api/v1/events` | Server-Sent Events: `snapshot` carries the `overview` JSON now and on every change |
| `GET /api/v1/auth/sessions` | the logged-in devices `{sessions: [{id, name, wallet, created_ms, last_used_ms, allowed}], current}`; `DELETE /api/v1/auth/sessions/<id>` revokes one |

Sign requests need no session (the id is the capability): `GET /api/v1/sign/{id}` → `{id, kind,
summary, message, expires_ms, state, opened, result?, error?}` (`kind` `login`, `change`, `rollback`,
`unlock`; `state` `pending`, `applying`, `unknown`, `done`, `expired`, `failed`); `POST
/api/v1/sign/{id}` `{wallet?, signature}` → `{state, result | error}` (404 unknown, 410 expired or
used, 403 bad signature, 409 the action failed); `POST /api/v1/sign/{id}/opened`, the signing page's
load ping, → 204 (404 unknown); 60 calls a minute per caller and request. Links follow the link rule in
[Sign requests](AUTH.md#sign-requests).

Device login needs no session: `POST /api/v1/auth/device` `{name?}` → `{code, poll_secret,
url, sign, expires_in, interval}` (`url` is the login's sign request link); `GET /api/v1/auth/device/<code>` → `{code, device, message,
expires_at, origin}` (the message to sign, `{wallet}` for the address); `POST
/api/v1/auth/device/<code>/approve` `{wallet, signature}`; `POST /api/v1/auth/device/token`
`{code, poll_secret}` → `{status: "pending"}`, `410 {status: "expired"}` or once `{status:
"approved", token, id, wallet, name}`.

`instance` picks one instance; leave it out for the only (or first) one. A paper bot shows
`snapshot.dry_run = true` and `account.instances[].mode = "paper"`.

## Everything configurable, and how to set it

`/api/v1/catalog` lists every setting of every installed strategy and exchange, so a client needs no
knowledge of any of them; the same read model backs the MCP tools (`catalog`, `describe`,
`strategies`). Values are the boot config's tables with the live overrides on top. A key or address
(an *input*) shows as `{name, env, set}` and never with a value. Page through large catalogs with `page`;
`total` counts matching settings.

A setting is changed through the ordinary change endpoints (`/ui/api/propose`, `/ui/api/apply`) or MCP
(`propose_change`, `apply_change`) with its path as the `set` key, the value as text:

- `strategies.<strategy>.<setting>`, e.g. `strategies.grid.lower_price`, `strategies.funding-arb.hedge_exchange`,
  `strategies.grid.fees.maker_bps` (a dotted setting path);
- `exchanges.<venue>.<key>`, e.g. `exchanges.hyperliquid.network`;
- the existing `strategy`, `symbol`, `mode`, `params.<name>`, `guards.<name>`, `limits.<name>`.

Each setting's `tier` says how it is applied; the safety gate decides what needs the owner's signature, unchanged.
Secrets are never a path: the owner authorizes the bot's own key by its address in the venue's app.

## Changing things

`/api/v1` is read-only. The two exceptions start and drop a paper twin and change nothing the bot does:
`POST /ui/api/stage` (`{changes: {name: "number"}, reason}`) and `POST /ui/api/stage/clear`.
`GET /ui/api/config/options` lists the strategies, params with their ranges, and limits; `POST
/ui/api/config/preview` checks a change (nothing applied or signed). Changes go through `/ui/api`
(propose, sign with the wallet, apply), where the safety gate decides what needs a signature. A change,
rollback or unlock that needs one opens a [sign request](AUTH.md#sign-requests) and answers with its
`sign` link.
