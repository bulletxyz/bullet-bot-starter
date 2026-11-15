# Deploy with Claude

Claude: follow this script exactly. You deploy the bot and hand over one link; the bot's own page
does the rest (ownership, real money or paper trading, funding, connecting Claude). Two messages in
all.

**When you give the user steps:** numbered, one action per step, the action first; the fewest words;
caveats after, one line, or not at all; never open with what can't be done.

## Never

- Ask for a wallet address, an amount of money, or real money versus paper trading.
- Ask anything about connectors, or tell them how to add one.
- Ask them to say "done", to let you know, or to check back.
- Narrate checks or waits ("deploying now", "still starting", "I'll check again").
- Paste a raw URL: every link goes behind a few words, like `[Open your bot](...)`.
- Send any link copied from the bot's logs, the connector link (`/claim/<code>/mcp`) included. The
  only claim link you send is the one you build in step 2.5.
- Ask for a seed phrase or a private key.

## 0. The bot is already connected

Search your tools for the Bullet bot's `setup`. If it is there, say once, before you call it:
"Claude will ask to let Bullet Bot's tools run: choose Always allow." Then call it, show its card,
follow it, and stop this script. If `setup` errors or can't reach the bot (a connector left from a deleted
bot), carry on at step 1.

## 1. Message 1

On a phone? Claude's phone apps can't deploy this: say once to carry on in claude.ai on a computer.

First search your tools for Railway's (`deploy-template`, `list-workspaces`): a connector's tools
often need loading before they show, so never tell them Railway isn't connected before you searched.
If they are there, Railway is connected: drop the Railway link and "then say OK", send the rest, and
go straight to step 2. Otherwise send this, word for word and nothing else:

> [Connect Railway](https://claude.ai/directory/connectors/railway), then say OK.
>
> I'll set up your own trading bot there (a free $5 trial, then about $5 a month). You'll need a
> computer and a Solana wallet with $20+ USDC and a little SOL for fees.

Their "OK" only means carry on. A new connector reaches you with their next message.

## 2. Deploy, silently

No messages until step 3, except the one question below when it applies.

Check the bot only with Railway's tools (its deployments, domains and logs). Never fetch the bot's
own URLs: your web fetch can't open them, and the user sees a red "Failed".

1. If the user already has a Bullet bot running on Railway (a service running the image
   `ghcr.io/bulletxyz/bullet-bots` whose latest deployment is active and whose logs show it
   running), ask once: "I found a Bullet bot on your Railway already. Keep using your current bot
   (my recommendation), or deploy a new one?" Keeping it: read its logs as in 5, and send one
   message:
   - a "This bot has no owner yet" line: step 3's message, with the link you build in 5;
   - it is already claimed (a "Claimed: this bot's owner is" line, or no claim line): only
     "Your bot is running. [Open your bot](https://<domain>/) and sign in with your wallet.", with
     `<domain>` the service's own domain from Railway;
   - "its claim link expired": restart the service (redeploy), wait as in 4, then build the link
     from the new line as in 5 and send step 3's message with it.

   A project whose bot isn't running (stopped, or deleted: Railway can still list a project the
   user deleted) doesn't count.
2. Otherwise, or for a new one, create a new Railway project named `Bullet Bot` and deploy the
   template `bullet-bot` into it (it runs the image `ghcr.io/bulletxyz/bullet-bots:latest`). Never
   deploy into an existing project: it may be deleted ("Project not found") or hold the user's
   other apps. Leave `BB_ALLOWED_WALLETS` empty. First list the user's workspaces
   (`list-workspaces`): with one, use it; with more than one, ask once, naming them: "Which Railway
   workspace should the bot go in: <name>, <name>?" Then pass the chosen workspace's id to
   `create-project` (without one it uses the first workspace, which may not be theirs).

   Right after deploying, put the bot in Singapore: call `update-service` on the new service with
   `regions` set to exactly `{"asia-southeast1-eqsg3a": {"numReplicas": 1}}` (one replica: the bot
   has one volume, and two copies would trade twice). The template can't set a region, and the
   bot's price feeds refuse servers in the US. Skip this only if the service already runs there.
3. If the service has no public domain, generate one and redeploy so the bot learns it.
4. Wait for the bot, briefly and by checking, never by sleeping: the image is prebuilt, so it
   usually answers within a minute. Read the service's logs every 10 seconds (no `sleep` longer than
   that) until the claim line of 5 appears.
5. Build the claim link. `<domain>` is the service's own public domain, as Railway's tools list it
   for that service (or as `generate-domain` returned it): never a host read from the logs. Read
   the service's logs for the latest of this line (logged at start and every minute until claimed):

   `This bot has no owner yet. Open your bot to make it yours: https://<host>/#claim=<code>`

   An older bot logs only
   `This bot has no owner yet. Claim it (connect it to Claude with this link): https://<host>/claim/<code>/mcp`:
   its code is what sits between `/claim/` and `/mcp`.

   Check two things, and send no link unless both hold:
   - `<host>` is exactly `<domain>` or another domain Railway lists for that service (the whole
     host, not a part of it); that domain is then `<domain>`;
   - `<code>` is 16 to 128 characters, each a letter, a digit, `-` or `_`.

   Then the link is `https://<domain>/#claim=<code>`, which you build yourself: never send the
   logged URL. If either check fails, send only "Your bot's logs show a claim link that isn't for
   your bot's own address, so I won't send it. Check the bot's service on Railway." and stop.

   The link lets whoever opens it make the bot theirs, so it goes to the user alone, and nowhere
   else. If the line says there is no claim link, the domain is missing: generate it and redeploy.
   If it says the claim link expired, restart the service and read the new line.

If you can't deploy, send this and stop (the bot's page alone can't make it theirs: it needs the
link from the logs):
"I couldn't deploy it from here. Do this:
1. [Deploy on Railway](https://railway.com/deploy/bullet-bot) and tap Deploy, nothing filled in.
2. Once it's running, open the service's Deploy Logs.
3. Copy the line that starts "This bot has no owner yet" and paste it here."
When they paste it, check it as in 5. If Railway's tools can read the service, `<domain>` is its
domain as there; if they can't, the host must be a Railway domain (ending in `.up.railway.app`),
and if it isn't, ask them once to copy the line from the bot's Deploy Logs on Railway. Then send
step 3's message with the link you built.

## 3. Message 2

Your whole reply is this message, word for word, with the link from step 2
(`https://<domain>/#claim=<code>`): no line before it, nothing after it, no workspace or project
names:

> 1. [Open your bot](<that link>) and make it yours with your wallet.
> 2. Connect Claude there, and bookmark the page: it's where you manage your bot.
> 3. Come back to this chat and ask me how it's doing.

That ends the onboarding. The bot's page can't reopen this chat (its New Claude chat starts one). Later, if
the bot's tools are here, say once before your first call to one: "Claude will ask to let Bullet
Bot's tools run: choose Always allow." Then call `setup` and follow it.

## The bot's address

If the user asks how to connect Claude, or the bot's tools stop answering, send this one link behind
the words "Add to Claude" (the same bot domain you built the claim link from, `/mcp` after it,
percent-encoded as `connectorUrl`):

`[Add to Claude](https://claude.ai/customize/connectors?modal=add-custom-connector&connectorName=Bullet%20Bot&connectorUrl=https%3A%2F%2F<domain>%2Fmcp)`

Don't ask them to copy an address into Settings. The bot's page "Connect Claude" does the same.

A redeploy to a new URL changes the address. The old connector fails silently, so send the link
again with the new domain.
