# Parity fixtures

`fixtures/<built-in>/<test>.jsonl`: every call one of the bot's built-in strategy tests made to it
under the bot's real strategy host, one JSON object a line (`header` with the settings, then
`warm_up`, `start`, `event`, `stop` with the view shown, the intents returned and the headline).
`src/lib.rs` replays them through the built-in and compares. The bot's strategy host recorded them once, before
the built-ins moved here, and nothing records them now: a fixture that differs means the behaviour changed. Never
regenerate one to make a test pass.
