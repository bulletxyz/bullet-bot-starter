# Funding arb: strategy vs safety executor

Multi-symbol Bullet x Hyperliquid funding arb. The settings are `[strategies.funding-arb]` on the bot.

The funding arb has two halves with different rules; only the first is in this repository.

- **The strategy: this crate, `src/strategy.rs`.** The pure `decide` (a `MarketView` in, one `Target` per symbol
  out) and `FundingArb`, a plain `bb_strategy_sdk::StrategyAdapter`: it answers `Event::MarketView` with
  `Intent::Targets` and nothing else. **Change behaviour here**: entry/exit rules, sizing, ranking. `decide` does no
  I/O and never reaches a venue. Signal and cost helpers (`trailing_median`, `walk_slip_bps`) are in
  `bb-strategy-support`.
- **The safety executor** is part of the bot, not of this repository. It builds the `MarketView`, clamps whatever
  `decide` returns to its caps, executes and hedges, reconciles with the venues, flattens and holds when stuck. A
  strategy bug can size a position wrongly but cannot exceed a limit or trade on a failed read. A host that has no
  executor never delivers `Event::MarketView`, so the strategy then does nothing.

Prefer a change in `src/strategy.rs`; if a limit really is wrong, say so and propose the new value and reason.

`tests/adapter.rs` checks the strategy on its own. The executor's tests decide with a frozen copy
of `decide` in the bot: a deliberate change to `decide` here must be copied there in the same change, so say so in
your pull request.
