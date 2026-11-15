# Bullet bot starter

Your own trading bot on [Bullet](https://bullet.xyz) perpetuals, run from Claude, and the kit to
write your own strategies for it.

## 1. Get your bot

[![Deploy with Claude Code][badge]][deploy]

[badge]: https://img.shields.io/badge/Deploy%20with-Claude%20Code-D97757?logo=claude&logoColor=white
[deploy]: https://claude.ai/code?prompt=Deploy%20and%20set%20up%20my%20Bullet%20trading%20bot%20for%20me.%20Follow%20this%20guide%3A%20https%3A%2F%2Fraw.githubusercontent.com%2Fbulletxyz%2Fbullet-bot-starter%2Fmain%2FDEPLOY-WITH-CLAUDE.md

Use Claude Code, not a regular Claude chat: the button opens it (in the Claude app, the **Code** tab). No message
filled in? Paste this into Claude Code:

```text
Deploy and set up my Bullet trading bot for me. Follow this guide: https://raw.githubusercontent.com/bulletxyz/bullet-bot-starter/main/DEPLOY-WITH-CLAUDE.md
```

You need a Claude plan with Claude Code (Pro or above), a computer, and a Solana wallet with $20+ USDC and a
little SOL for fees. No wallet yet?
[Phantom](https://phantom.com/download) takes two minutes.

1. **Click Deploy with Claude Code, then send it in Claude Code.** Connect Railway when Claude asks, and say OK. Claude
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

[![Build a strategy with Claude Code][badge2]][build]

[badge2]: https://img.shields.io/badge/Build%20a%20strategy-Claude%20Code-D97757?logo=claude&logoColor=white
[build]: https://claude.ai/code?prompt=Build%20a%20strategy%20plugin%20for%20my%20Bullet%20bot%20and%20install%20it%20on%20paper.%20Follow%20this%20guide%3A%20https%3A%2F%2Fraw.githubusercontent.com%2Fbulletxyz%2Fbullet-bot-starter%2Fmain%2Fskills%2Fbullet-wasm-plugin%2FSKILL.md

Use Claude Code, not a regular Claude chat: the button opens it (in the Claude app, the **Code** tab). No message
filled in? Paste this into Claude Code:

```text
Build a strategy plugin for my Bullet bot and install it on paper. Follow this guide: https://raw.githubusercontent.com/bulletxyz/bullet-bot-starter/main/skills/bullet-wasm-plugin/SKILL.md
```

1. Connect your bot to Claude first (step 1).
2. Click the button and send it in Claude Code, then say what your strategy should do. Claude Code builds it
   from this repo's template.
3. If Claude asks, allow your bot's address in network access (a few clicks, it shows you how, from
   [claude.ai/code](https://claude.ai/code); that setting has no direct link).
4. Approve the plugin with your wallet. It starts on paper.

By hand instead: [Make a strategy](docs/MAKE-A-STRATEGY.md), then [install it](docs/INSTALL.md).

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
| [strategies/](strategies/) | Example plugins: [ladder](strategies/ladder/), and the bot's grid and RSI momentum ported |
| [templates/strategy/](templates/strategy/) | What a new strategy starts from |
| [sdk/](sdk/) | The strategy SDK ([cheat sheet](docs/SDK.md)) |
| [skills/bullet-wasm-plugin/](skills/bullet-wasm-plugin/SKILL.md) | The guide Claude follows to build and install a plugin |

## Risk

> This is software that trades real money: you can lose money, and the losses are yours. Start
> small, on paper. Bullet's terms apply to anything you trade there. The bot's key can never
> withdraw, but it trades your whole Bullet account, so a loss can reach all of it.

Licensed under [Apache-2.0](LICENSE). No warranty of any kind. Not financial advice.
