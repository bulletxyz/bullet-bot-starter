# Bullet bot starter

Your own trading bot on [Bullet](https://bullet.xyz) perpetuals, run from Claude, and the kit to
write your own strategies for it.

## 1. Get your bot

### Deploy to Render

[![Deploy to Render](https://render.com/images/deploy-to-render-button.svg)][render-deploy]

[render-deploy]: https://render.com/deploy?repo=https://github.com/bulletxyz/bullet-bot-starter

1. **Click Deploy to Render** and approve the paid service. Render starts your bot in Singapore.
2. **Open Logs** for `bullet-bot` in Render and look for **Open your bot to make it yours**.
3. **Open that link** and use your Solana wallet to claim your bot.
4. **Choose Paper trading** to try it without funding it.

Keep the claim link private. Connect Claude from your bot’s page if you want to manage it from chat.

### Or deploy directly on Railway

**Set Singapore before you deploy.** The template uses your workspace’s preferred region.

1. **Open [Railway workspace settings](https://railway.com/workspace)** for the workspace you’ll use.
2. **Set Preferred Deployment Region** to **Southeast Asia (Singapore)** and click **Update Region**.
3. **Click Deploy to Railway** below and choose that same workspace.
4. **Verify Singapore** for both the service and its `/data` volume before claiming the bot.
5. **Open Logs** for `bullet-bot` and look for **Open your bot to make it yours**.
6. **Open that link** and use your Solana wallet to claim your bot.
7. **Choose Paper trading** to try it without funding it.

[![Deploy to Railway][railway-badge]][railway-deploy]

[railway-badge]: https://img.shields.io/badge/Deploy%20to-Railway-0B0D0E?logo=railway&logoColor=white
[railway-deploy]: https://railway.com/deploy/bullet-bot

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
[build]: https://bulletxyz.github.io/bullet-bot-starter/build/

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
| [strategies/](strategies/) | Example plugins: [ladder](strategies/ladder/), [sma](strategies/sma/) |
| [strategies/](strategies/) | The bot's built-ins (grid, RSI momentum, Avellaneda-Stoikov, funding), not plugins |
| [templates/strategy/](templates/strategy/) | What a new strategy starts from |
| [sdk/](sdk/) | The strategy SDK ([cheat sheet](docs/SDK.md)) |
| [skills/bullet-wasm-plugin/](skills/bullet-wasm-plugin/SKILL.md) | The guide Claude follows to build and install a plugin |

## Risk

> This is software that trades real money: you can lose money, and the losses are yours. Start
> small, on paper. Bullet's terms apply to anything you trade there. The bot's key can never
> withdraw, but it trades your whole Bullet account, so a loss can reach all of it.

Licensed under [Apache-2.0](LICENSE). No warranty of any kind. Not financial advice.
