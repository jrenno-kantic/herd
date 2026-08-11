# Data Model

## App

- `command_input: String`
- `screen: Screen` - `Models` | `Server` | `Test` | `Stats` | `Settings` | `Logs`
- `mode: Mode` - `Browse` | `Command` | `Filter` | `EditSetting` | `EditPrompt` | `Picker` | `ConfirmLaunch`
- `logs: VecDeque<String>` - capped at 500 entries, oldest dropped first
- `running: bool` - a command is in flight; input is ignored while true
- `llama: LauncherState`

## LauncherState

- `config_path: PathBuf`, `config: Option<LlamaConfig>`, `config_error: Option<String>`
- `tiers: Vec<Tier>`, `ram_gib: Option<u64>`
- `cursor: usize`, `filter: String`
- `overrides: Overrides` - session-only
- `settings_cursor: usize`, `edit_buffer: String`
- `server: ServerRuntime`
- `pending_launch: Option<String>`, `port_conflict: Option<u16>`
- `last_launched: Option<String>`
- `prompt: String`, `chat: Option<Result<ChatOutcome, String>>`, `chat_pending: bool`
- `stats: SessionStats`, `reserved_ratio: f64`, `picker_cursor: usize`

## SessionStats

Reset on every launch. `started_at: Option<DateTime<Local>>`, `probes`,
`failures`, `prompt_tokens`, `completion_tokens`, `total_latency`, `last_rate`,
`best_rate`. `average_rate()` is total tokens over total time, not a mean of
per-request rates.

## Budget / Fit

`Budget { total_gib, reserved_ratio }` yields `available_gib()`. `Fit` is
`Fits` | `Tight` | `TooLarge` | `Unknown`; `Unknown` covers both an unparseable
preset name and an unreadable RAM figure, and is never rendered as a warning.

## ConfigChoice

`label`, `path`, `presets`, `too_large` - one row of the `models.ini` picker.

## ChatOutcome

- `model`, `prompt`, `reply`
- `latency: Duration` - measured locally, always present
- `prompt_tokens` / `completion_tokens: Option<u64>` - from `usage`, optional
- `tokens_per_second: Option<f64>` - llama.cpp `timings`, else derived from
  tokens and measured latency

## ServerRuntime

- `state: ServerState` - `Off` | `Starting` | `Serving` | `Stopping` | `Error(String)`
- `mode: LauncherMode` - `Idle` | `Router` | `Manual`
- `model: Option<String>`, `endpoint: Option<String>`, `started_at: Option<Instant>`

## LlamaConfig (parsed models.ini)

- `path`, `server: Section`, `defaults: Section`, `models: Vec<(String, Section)>`

A `Section` is an ordered `key = value` list: re-setting a key replaces its value
but keeps its original position.

## Overrides (session-only)

Two maps, `Global` (covering `[server]` and `[*]`) and per-model. Flattened to
argv and injected into the CLI-override slot, so the precedence chain gains a
step without the engine gaining a rule:

```
[server] -> [*] -> [model] -> overrides -> explicit CLI args
```

## Session (persisted)

`~/.config/herd/session.json`, holding **only** `config_path` and `model`.
Settings overrides are deliberately excluded.

## Shipped preset data (`data/`)

An in-repo snapshot of the user's `~/models/` tiers: `16gb/models.ini`
(12 presets) and `32gb/models.ini` (8 presets), plus the original
`llama-launch.js` and `start-router.sh`.

Reference and **test fixture only** - config resolution reads `~/models/`, never
`data/`. The suite parses these files so every shipped preset is proven to parse
and to build a launchable argv.

## UiEvent

`Key` | `Tick` | `CommandFinished { command, output }` | `Log(String)` |
`LlamaStatus(LlamaSnapshot)` | `PortInUse { port, model }` |
`ChatResult(Box<Result<ChatOutcome, String>>)` | `Quit`

## Action

`None` | `Quit` | `RunCommand(String)` | `ConfigPathChanged(PathBuf)` |
`RunChat { model, prompt }`
