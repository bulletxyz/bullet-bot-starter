# Bullet bot starter

Your own trading bot on [Bullet](https://bullet.xyz) perpetuals, run from Claude, and the kit to
write your own strategies for it.

## 1. Get your bot

[![Deploy with Claude][badge]][deploy]

[badge]: https://img.shields.io/badge/Deploy%20with-Claude-D97757?logo=claude&logoColor=white
[deploy]: https://claude.ai/new?q=Deploy%20and%20set%20up%20my%20Bullet%20trading%20bot%20for%20me.%20Follow%20this%20guide%3A%20https%3A%2F%2Fraw.githubusercontent.com%2Fbulletxyz%2Fbullet-bot-starter%2Fmain%2FDEPLOY-WITH-CLAUDE.md

You need a computer and a Solana wallet with $20+ USDC and a little SOL for fees. No wallet yet?
[Phantom](https://phantom.com/download) takes two minutes.

1. **Click Deploy with Claude, then send.** Connect Railway when Claude asks, and say OK. Claude
   puts your bot on Railway (a free $5 trial, then about $5 a month).
2. **Open your bot** from Claude's link and make it yours with your wallet. Pick **Paper
   trading** to try it first.
3. **Connect to Claude** from your bot's page, and Allow.

About three minutes.
[What Claude does](https://github.com/bulletxyz/bullet-bot-starter/blob/main/DEPLOY-WITH-CLAUDE.md).
Rather run it yourself? [Run it with Docker](docs/DEPLOY.md).

## 2. Trade

On paper the bot trades on Bullet's real prices and sends no order. Then just ask Claude:

- "How's my bot doing?"
- "Stop if it loses $10 today."
- "Go live with $100." Your wallet signs it, and you can go back to paper anytime.

Built in: the market maker (start here), grid and RSI momentum.

## 3. Make your own strategy

[![Build a strategy with Claude][badge2]][build]

[badge2]: https://img.shields.io/badge/Build%20a%20strategy-Claude-D97757?logo=claude&logoColor=white
[build]: https://claude.ai/new?q=Build%20a%20strategy%20plugin%20for%20my%20Bullet%20bot%20and%20install%20it%20on%20paper.%20Follow%20this%20guide%3A%20https%3A%2F%2Fgithub.com%2Fbulletxyz%2Fbullet-bot-starter%2Fblob%2Fmain%2Fskills%2Fbullet-wasm-plugin%2FSKILL.md

Tell Claude what your strategy should do. Claude writes it from this repo's template, builds it
and sends it to your bot. Nothing runs until your wallet signs it, and it starts on paper.

Before you start: your bot is connected to Claude (step 1), and in Claude on the web or desktop,
*Settings > Capabilities > Code execution* allows package managers, `static.rust-lang.org`,
`github.com` and your bot's address.

On your own computer instead: [Make a strategy](docs/MAKE-A-STRATEGY.md), then
[install it](docs/INSTALL.md).

## How your money stays safe

- **Paper first.** Until you go live, no order is sent.
- **It can't withdraw.** The bot trades with its own key, which can place orders on your Bullet
  account and never move money out. You can remove it any time.
- **Your wallet decides.** Going live, raising limits and new strategy code need your wallet's
  signature.
- **Hard limits.** A largest position and a daily loss stop hold for every strategy, yours too.

## What's here

| Folder | What |
|---|---|
| [strategies/](strategies/) | Example plugins: [ladder](strategies/ladder/), the bot's grid and RSI momentum ported, and three more you can copy: [Avellaneda-Stoikov](strategies/avellaneda-stoikov-wasm/), [simple funding](strategies/simple-funding-wasm/) and the [funding arb](strategies/funding-arb-wasm/) (the last two run on less than the bot's own versions: each crate's README lists what they cannot do) |
| [templates/strategy/](templates/strategy/) | What a new strategy starts from |
| [sdk/](sdk/) | The strategy SDK |
| [skills/bullet-wasm-plugin/](skills/bullet-wasm-plugin/SKILL.md) | The guide Claude follows to build and install a plugin |

## Risk

> This is software that trades real money: you can lose money, and the losses are yours. Start
> small, on paper. Bullet's terms apply to anything you trade there. The bot's key can never
> withdraw, but it trades your whole Bullet account, so a loss can reach all of it.

Licensed under [Apache-2.0](LICENSE). No warranty of any kind. Not financial advice.
