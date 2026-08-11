# Data Model

## App

- `command_input: String`
- `screen: Screen` - `Models` | `Server` | `Test` | `Stats` | `Settings` | `Logs`
- `mode: Mode` - `Browse` | `Command` | `Filter` | `EditSetting` | `EditPrompt` | `Picker` | `ConfirmLaunch` | `ConfirmQuit` | `Help`
- `logs: VecDeque<String>` - capped at 500 entries, oldest dropped first
- `log_scroll: usize` - lines hidden *below* the viewport, so 0 means "follow
  the newest line" and no separate follow flag is needed
- `running: bool` - a command is in flight; input is ignored while true, except
  for `stop`, which is the one command needed when something else is wedged
- `rows: u16` - last terminal height, from `UiEvent::Resize`; drives `page()`
- `llama: LauncherState`

## LauncherState

- `config_path: PathBuf`, `config: Option<LlamaConfig>`, `config_error: Option<String>`
- `tiers: Vec<Tier>`, `ram_gib: Option<u64>`
- `cursor: usize`, `filter: String`
- `overrides: Overrides` - session-only
- `settings_cursor: usize`, `edit_buffer: String`
- `server: ServerRuntime`
- `pending_launch: Option<String>`, `confirm: Option<Confirm>`
- `cached: Option<Vec<String>>` - what `llama-server --cache-list` last
  reported; `None` means "not asked yet", which is why availability answers
  `Unknown` rather than guessing
- `download: Option<Download>` - the fetch in flight
- `last_launched: Option<String>`
- `prompt: String`, `chat: Option<Result<ChatOutcome, String>>`, `chat_pending: bool`
- `stats: SessionStats`, `reserved_ratio: f64`, `picker_cursor: usize`
- `chat_started: Option<Instant>` - so the Test screen can count up while waiting

## Confirm

Why the launcher is asking before it launches:
`PortInUse(u16)` | `TooLarge { estimate, budget }` | `NotDownloaded { repo }`.

## Download

`model`, `done: u64`, `total: u64` — bytes rather than a percentage, so the
gauge and the "2.1G of 6.7G" beside it cannot disagree. `total == 0` means the
file list has not come back yet and renders as text, not a gauge stuck at zero.

## Availability

`Local` | `Missing` | `Unknown`, from `llama-server --cache-list` rather than a
filesystem check — llama.cpp is what has to load the file, and it correctly
refuses to list a repo whose current revision is half-downloaded.

## Optimisation / Capability

`Optimisation` is `Qat` | `Dynamic` | `MixtureOfExperts`, read from the repo
reference. `Capability` is `Vision` | `Speculative` | `Audio` | `Code`, carried
on a `Trait { capability, enabled, detail }` — `enabled` distinguishes "the
model has it" from "this preset uses it", and `detail` names the mechanism
(`speculative decoding (mtp)`).

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
- `sent_at: DateTime<Local>` - measured locally, always present
- `latency: Duration` - measured locally, always present
- `prompt_tokens` / `completion_tokens: Option<u64>` - from `usage`, optional
- `tokens_per_second: Option<f64>` - llama.cpp `timings`, else derived from
  tokens and measured latency
- `prompt_ms` / `predicted_ms: Option<f64>` - llama.cpp's own split of the round
  trip; absent from every other server, and degrades to nothing rather than to
  zeroes

## ServerRuntime

- `state: ServerState` - `Off` | `Starting` | `Serving` | `Stopping` | `Error(String)`
- `mode: LauncherMode` - `Idle` | `Router` | `Manual`
- `phase: Phase` - `None` | `Binding` | `Loading` | `Downloading(u64)` |
  `Unresponsive(u32)`. Sub-state rather than more `ServerState` variants:
  `is_live()` means "would a stop do something", and that answer is already
  right for every phase
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
(13 presets) and `32gb/models.ini` (8 presets). The original `llama-launch.js`,
`test_call.sh` and `start-router.sh` were kept here at first and have since been
removed — the Rust code is the source of truth for all three.

Reference and **test fixture only** - config resolution reads `~/models/`, never
`data/`. The suite parses these files so every shipped preset is proven to parse
and to build a launchable argv.

## UiEvent

`Key` | `Tick` | `CommandFinished { command, output }` | `Log(String)` |
`LlamaStatus(LlamaSnapshot)` | `PortInUse { port, model }` |
`ChatResult(Box<Result<ChatOutcome, String>>)` | `CacheList(Vec<String>)` |
`DownloadProgress { model, done, total }` |
`DownloadFinished { model, result }` | `Resize { height }` | `Quit`

## Action

`None` | `Quit` | `RunCommand(String)` | `ConfigPathChanged(PathBuf)` |
`RunChat { model, prompt }` |
`Download { model, repo, wants, then_launch }`
