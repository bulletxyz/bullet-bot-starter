# Deploy with Claude

Claude: follow this script exactly, section by section. You put the user's trading bot on Railway in
Singapore and send them one link. The bot's own page does the rest (ownership, paper or real money,
funding, connecting Claude). Normally the user gets two messages from you: Message 1 (section B)
and Message 2 (section D). The other texts below are for the cases they name.

If a step needs a Railway action you have no tool for (reading deployments or regions, creating a
service or volume, deleting a project), give exactly that action to `railway-agent` with the ids and
exact values, ask it to return the ids of anything it creates, and use what it reports.

## Rules

- Send only the messages this script gives you, word for word. Don't narrate checks or waits.
- Put every link behind a few words, like `[Open your bot](...)`: never a raw URL, never a
  placeholder for the user to fill in.
- Never ask for a wallet address, an amount of money, paper versus real money, a seed phrase, a
  private key or a Railway token (Railway's connector signs in by itself).
- Never ask about connectors, or ask the user to say "done" or check back (Message 1's "then say OK"
  is the one exception).
- The only claim link you ever send is the one you build in C5. Never send a link from the logs.
- Never open the bot's own URLs with web fetch: it can't, and the user sees a red "Failed". Check the
  bot only through Railway's tools (services, deployments, domains, logs).

## A. Is the bot already connected?

Search your tools for the Bullet bot's `setup`. If it's there: say "Claude will ask to let Bullet
Bot's tools run: choose Always allow.", call `setup`, follow what it returns, and stop this script.
If `setup` is missing, errors or can't reach the bot, go to B.

## B. Message 1: Railway

Search your tools for Railway's (`list-workspaces`, `create-project`). Connector tools often need
loading first: always search before deciding Railway isn't connected.

- **Railway's tools are there:** send this, then go straight on to C in the same turn:

  > I'll set up your own trading bot on Railway (a free $5 trial, then about $5 a month). You'll
  > need a computer and a Solana wallet with $20+ USDC and a little SOL for fees.

- **They aren't:** send this and end your turn:

  > [Connect Railway](https://claude.ai/directory/connectors/railway), then say OK.
  >
  > I'll set up your own trading bot there (a free $5 trial, then about $5 a month). You'll need a
  > computer and a Solana wallet with $20+ USDC and a little SOL for fees.

  When the user replies (any reply), search again: the tools arrive with their reply. Found: go to
  C. Still missing: go to C6.

If the user has said they're on a phone, end Message 1 with: "The bot's page needs your wallet on a
computer."

## C. Deploy (silently)

Send nothing in this section except the questions in C1 and C2, and only when they apply.

**The bot must be in Singapore (`asia-southeast1-eqsg3a`) from its very first deployment.** Its
price feeds refuse US servers. Railway puts a new service and its volume in the workspace's default
region (usually the US), and a volume can't follow a later region change. So:

- never deploy the `bullet-bot` template yourself, nor use `deploy-template` (they deploy at once,
  in the default region);
- never deploy and then move it;
- build the service yourself as in C3, and deploy only at its last step.

### C1. Look for a bot the user already has

List the user's Railway projects and services. A bot is a service whose image starts with
`ghcr.io/bulletxyz/bullet-bots`; it is running if its latest deployment is active. Ignore bots that
aren't running. For a running one, read its region:

- not in the US (not `us-…`): it counts;
- in the US: it can't trade. Leave it alone, note its domain (from Railway, not its logs), and add
  Message 2's last line for it;
- unknown: ignore it.

None counts: go to C2. One counts: ask "I found a Bullet bot on your Railway already. Keep using
your current bot (my recommendation), or deploy a new one?"

- **New one:** go to C2.
- **Keep it:** read its logs as in C5:
  - a "This bot has no owner yet" line: build the link (C5) and send Message 2 (D);
  - "the claim link expired": redeploy the service, then C4 and C5, then Message 2;
  - otherwise (a "Claimed: this bot's owner is" line, or no claim line): send only "Your bot is
    running. [Open your bot](https://<domain>/) and sign in with your wallet.", where `<domain>` is
    the service's domain on Railway. Stop.

### C2. Make the project

1. `list-workspaces`. One workspace: use it. More than one: ask "Which Railway workspace should the
   bot go in: <name>, <name>?"
2. `create-project` named `Bullet Bot`, passing that workspace's id (without it Railway picks the
   first workspace, which may not be theirs). Never use an existing project. Another project may
   have the same name: from here on, use only the project id `create-project` returned.

### C3. Build the service in Singapore

Do these in order, one call each: never combine two steps in one call. Use your own Railway tool
for a step when there is one; otherwise `railway-agent`, adding "Do not deploy." for steps 1 to 4
(the service has no image yet, so nothing should deploy).

1. Create a service named `bullet-bot` in the new project with **no source**. Read its
   deployments: any at all, go straight to the failure rule below.
2. `update-service` with `regions` set to exactly `{"asia-southeast1-eqsg3a": {"numReplicas": 1}}`.
   One replica only: two copies would trade twice.
3. Add a volume to the service, mounted at `/data`. A new volume takes its service's region.
4. Generate the service's public domain. This is `<domain>`.
5. **Check before the image goes on.** Read the service's deployments, its regions and the volume's
   region. Go on only if there are no deployments and both regions are `asia-southeast1-eqsg3a`
   alone (a volume region no tool can read: go on).
6. Set the service's source to the image `ghcr.io/bulletxyz/bullet-bots:latest`. Add no variables.
7. Read the deployments. One exists (step 6 started it): go to C4. None: `accept-deploy`, then
   read them again; still none: `redeploy`. Still none: delete the project and go to C6.

If the check in step 1 or 5 fails: delete the project (by its id) and start again at C2 step 2, in
the same workspace without asking again. Never migrate it: nobody owns that bot yet, so nothing is
lost. If the restarted attempt fails too, or a step other than a delete can't be done even by
`railway-agent`: delete that project and go to C6.

If a delete fails, don't retry it and don't restart: go to C6 now, and end its message with "Then
delete the empty `Bullet Bot` project(s) I made just now (none of them has a bot running)."

### C4. Wait for the bot

Read the logs of the service's newest deployment (older deployments' lines don't count) until a
"This bot has no owner yet" line appears, about every 10 seconds (`sleep 10` at most where you
have a shell; with no way to wait, just read again, up to 30 times). It usually takes under a
minute. If the deployment fails, or 5 minutes pass without the line, go to C6.

### C5. Build the claim link

`<domain>` is the service's public domain as Railway lists it: never a host taken from the logs.
Read the newest "This bot has no owner yet" line from the newest deployment's logs:

- it says there's no claim link: the domain is missing. Generate one, redeploy, back to C4;
- it says the claim link expired: redeploy, back to C4;
- otherwise it is one of these:
  - `This bot has no owner yet. Open your bot to make it yours: https://<host>/#claim=<code>`
  - an older bot: `This bot has no owner yet. Claim it (connect it to Claude with this link):
    https://<host>/claim/<code>/mcp` (the code sits between `/claim/` and `/mcp`).

Redeploy at most once in C5; if you'd need a second, go to C6.

Both must hold:

- `<host>` is exactly a domain Railway lists for this service (the whole host);
- `<code>` is 16 to 128 characters, each a letter, digit, `-` or `_`.

If both hold, the link is `https://<domain>/#claim=<code>`: go to D. It lets whoever opens it own
the bot, so send it to the user only.

If either fails, don't redeploy and don't build a link: send only "Your bot's logs show a claim
link that isn't for your bot's own address, so I won't send it. Check the bot's service on
[Railway](https://railway.com/dashboard)." and stop.

### C6. If you can't deploy at all

Send this and stop:

> I couldn't deploy it from here. Do this:
> 1. In your [Railway workspace settings](https://railway.com/workspace), set the default region to
>    Southeast Asia (Singapore).
> 2. [Deploy on Railway](https://railway.com/deploy/bullet-bot) and tap Deploy, nothing filled in.
> 3. Once it's running, open it in your [Railway dashboard](https://railway.com/dashboard), then
>    the service's Deploy Logs.
> 4. Copy the line that starts "This bot has no owner yet" and paste it here.
>
> Step 1 comes first: the bot can't trade from the US.

(The template is fine here only because step 1 changes the default region first.)

When they paste the line, check it as in C5. If Railway's tools can't read the service, `<host>`
must end in `.up.railway.app` and is then `<domain>`; if it doesn't, ask once for the line from the
bot's Deploy Logs. Then send Message 2.

## D. Message 2

This ends your reply, with the link from C5. Nothing else goes before it except Message 1 (when
you sent it in this same turn), and nothing after it except the optional line below:

> 1. [Open your bot](https://<domain>/#claim=<code>) and make it yours with your wallet.
> 2. Connect Claude there, and bookmark the page: it's where you manage your bot.
> 3. Come back to this chat and ask me how it's doing.

Only if C1 found a running bot in the US, add this last line, with that bot's domain (or, if it has
none, its project's name):

> Your older bot at <its domain> can't trade from the US: delete the project it's in on
> [Railway](https://railway.com/dashboard) (not the new one) to stop paying for it.

Onboarding ends here. Later in the chat, when the bot's tools are here, say once before your first
call to one: "Claude will ask to let Bullet Bot's tools run: choose Always allow." Then call `setup`
and follow it.

## Later: connecting Claude to the bot

If the user asks how to connect Claude, or the bot's tools stop answering, send this (`<domain>` as
in C5):

`[Add to Claude](https://claude.ai/customize/connectors?modal=add-custom-connector&connectorName=Bullet%20Bot&connectorUrl=https%3A%2F%2F<domain>%2Fmcp)`

Never ask them to copy an address into Settings. In a Claude Code terminal without the connector:
`claude mcp add --transport http bullet-bot https://<domain>/mcp`, then `/mcp` to sign in
([docs](https://code.claude.com/docs/en/mcp)). A connector added at claude.ai shows up in Claude Code
by itself.

If the domain changes, the old connector fails silently: send the link again with the new domain.
