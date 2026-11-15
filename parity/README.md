# Parity fixtures

`fixtures/<built-in>/<test>.jsonl`: every call one of the bot's built-in strategy tests made to it
under the bot's real strategy host, one JSON object a line (`header` with the settings, then
`warm_up`, `start`, `event`, `stop` with the view shown, the intents returned and the headline).
`src/lib.rs` replays them through a port and compares. The bot's maintainers record them from the built-ins' own tests.
