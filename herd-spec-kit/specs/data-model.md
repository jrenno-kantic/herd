# Data Model

## App

- `command_input: String`
- `screen: Screen` - `Models` | `Hub` | `Server` | `Router` | `Test` | `Stats` | `Settings` | `Logs`
- `mode: Mode` - `Browse` | `Command` | `Filter` | `EditSetting` | `EditPrompt` | `Picker` | `ConfirmLaunch` | `ConfirmQuit` | `ConfirmDelete` | `Help` | `Commands` | `About` | `OpenCode`
- `logs: VecDeque<String>` - capped at 500 entries, oldest dropped first
- `log_scroll: usize` - lines hidden *below* the viewport, so 0 means "follow
  the newest line" and no separate follow flag is needed
- `running: bool` - a command is in flight; input is ignored while true, except
  for `stop`, which is the one command needed when something else is wedged
- `in_flight() -> Vec<String>` (derived) - everything quitting right now would
  interrupt, each named: the download, a waiting chat probe, a running command.
  A live server is deliberately **not** in it — stopping it on exit is what
  happens every time, so `q` asks about it without listing it as work lost
- `rows: u16`, `cols: u16` - last terminal size, from `UiEvent::Resize`; drive
  `page()` and the argv preview's scroll bound
- `system: SystemInfo` - architecture, optional GPU name, and latest available
  memory sample for the sidebar
- `tools: Tools` - startup preflight results for `llama-server` and `hf`; the
  same state gates downloads and supplies the `:about` rows
- `llama: LauncherState`

## Tool / Tools

`Tool { name, version: Result<String, String> }` records either the first
non-empty line from `<tool> --version` or the actionable probe error. `Tools`
holds `llama_server` and `hf`; only the former contributes a
`required_error()`. The latter's `available()` value is the download capability
gate.

## LauncherState

- `config_path: PathBuf`, `config: Option<LlamaConfig>`, `config_error: Option<String>`
- `tiers: Vec<Tier>`, `ram_gib: Option<u64>`
- `cursor: usize`, `filter: String`
- `overrides: Overrides` - persisted in `~/.herd_config`
- `favorites: BTreeSet<String>` - starred presets, by name rather than by tier:
  the same preset appears in more than one tier and is the same model
- `router: RouterPrefs`, `router_cursor: usize` - the two numbers the Router
  screen would start llama-server's own router with
- `settings_cursor: usize`, `edit_buffer: String`
- `server: ServerRuntime`
- `pending_launch: Option<String>`, `confirm: Option<Confirm>`
- `cached: Option<Vec<CachedModel>>` - what `llama-server --cache-list` last
  reported, with the sizes measured off the cache directory; `None` means "not
  asked yet", which is why availability answers `Unknown` rather than guessing
- `hub_cursor: usize` - position in the Hub list
- `download: Option<Download>` - the fetch in flight
- `last_launched: Option<String>`
- `prompt: String`, `chat: Option<Result<ChatOutcome, String>>`, `chat_pending: bool`
- `stats: SessionStats`, `reserved_ratio: f64`, `picker_cursor: usize`
- `preview_scroll: usize` - lines of the argv preview hidden above its
  viewport; 0 is the top, and it is clamped in `App::update` against the
  pane's size so it can never climb past what is actually hidden
- `chat_started: Option<Instant>` - so the Test screen can count up while waiting

## Confirm

Why the launcher is asking before it launches:
`PortInUse { port, retry }` | `TooLarge { estimate, budget }` |
`NotDownloaded { repo }`. `retry` is the force command a yes re-dispatches
(`launch! …`, `router! …`), authored verbatim by the Executor that refused it.

## PendingDelete

The cached model awaiting an answer: `reference`, `repo`, `bytes`, and
`also_removes` — the other cached quantisations in the same repo directory,
resolved *before* the question is asked. A prompt that says "delete this model?"
and silently takes a second one with it has not asked the question the user
answered.

## Download

`model`, `done: u64`, `total: u64` — bytes rather than a percentage, so the
gauge and the "2.1G of 6.7G" beside it cannot disagree. `total == 0` means the
file list has not come back yet and renders as text, not a gauge stuck at zero.

## Availability

`Local` | `Missing` | `Unknown`, from `llama-server --cache-list` rather than a
filesystem check — llama.cpp is what has to load the file, and it correctly
refuses to list a repo whose current revision is half-downloaded.

## CachedModel

One entry of that listing, with **two** sizes because there are two questions:

- `reference` — `repo:quant`, as the cache spells it (which is not how the ini
  spells it: llama.cpp drops Unsloth's `UD-` prefix)
- `weights: Option<u64>` — what llama.cpp would load: this quantisation's gguf
  in the revision `refs/main` names, resolved through the snapshot symlinks
- `bytes: Option<u64>` — everything the repo's `blobs/` holds, stale revisions
  included. Per repo, since that is the unit the cache stores

`None` rather than `0` throughout: a directory that cannot be read and one that
is empty are different answers, and only one is worth printing.

## HubRow

A `CachedModel` joined to the active tier: `reference`, `weights`, `disk`,
`preset: Option<String>` (the preset naming that repo — `None` is what the
screen colours), and `shares_disk` (another cached quantisation lives in the
same directory, so the disk figure is not this model's alone).

## Sizing

`Measured(f64)` | `Estimated(f64)` — where a preset's size came from. Rendered
`8.1G` and `~8.1G`: once the weights are on disk there is a real file to read,
and the heuristic behind the estimate has been wrong by a factor of four.
A measurement only counts when the *quantisation* matches, unlike `Availability`,
which accepts a repo cached under any tag.

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

Time to first token is **three figures, and the leading one is cold**.
`first_token: Option<Duration>` is the first probe after the load and no other —
the only request that measures a model whose weights are not yet resident.
`last_ttft`, `ttft_probes` and `total_ttft` cover every probe that reported one
and describe the *warm* model: what a request costs once it is loaded.

They are kept apart rather than merged, since one mean over both drifts towards
the warm value the more probes are run and describes neither. All are reset on
every `Starting`, so a relaunch measures again. `first_token` stays `None` when
that first probe's server reported no `timings` — a later probe is warm, and
promoting one would be a warm number wearing a cold label — while the warm
figures still show. The warm counters have their own probe count rather than
using `probes`, because a server that sends no timings still answers.

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
- `ttft()` - `latency - predicted_ms`: the wait before the first token, derived
  because the probe is non-streaming on purpose. `None` without `timings`, and
  `None` when the subtraction goes negative, which is clock noise rather than a
  measurement

## ServerRuntime

- `state: ServerState` - `Off` | `Starting` | `Serving` | `Stopping` | `Error(String)`
- `mode: LauncherMode` - `Idle` | `Router` | `Manual`
- `phase: Phase` - `None` | `Binding` | `Loading` | `Downloading(u64)` |
  `Unresponsive(u32)`. Sub-state rather than more `ServerState` variants:
  `is_live()` means "would a stop do something", and that answer is already
  right for every phase
- `model: Option<String>`, `endpoint: Option<String>`, `started_at: Option<Instant>`

## LlamaConfig (parsed models.ini)

- `path`, `server: Section`, `defaults: Section`, `mono_focus: Section`,
  `models: Vec<(String, Section)>`

`mono-focus` is a **reserved section name**, like `server` and `*` — that is how
the profile is "handled". Every other section is a preset, so without reserving
it the profile would appear on the Models screen as a launchable model with no
`hf-repo`. `ini::is_reserved` is the single list.

## LaunchSettings

What a launch needs that the ini does not carry: the session `overrides`, and
`mono_focus` — the presets the profile is switched on for, by name.

`LaunchSettings::argv` is the **only** place a launch's argv is assembled. It
exists because the preview and the launch had diverged: `argv_preview` applied
the overrides and the Executor did not, so the Models screen drew flags the
spawned process never saw. Precedence:

```
[server] -> [*] -> [model] -> mono-focus -> overrides -> CLI
```

A `Section` is an ordered `key = value` list: re-setting a key replaces its value
but keeps its original position.

## Overrides

Two maps, `Global` (covering `[server]` and `[*]`) and per-model. Flattened to
argv and injected into the CLI-override slot, so the precedence chain gains a
step without the engine gaining a rule:

```
[server] -> [*] -> [model] -> overrides -> explicit CLI args
```

Persisted in `~/.herd_config`. The "never written to disk" rule was always
about the *ini*, which is hand-maintained and commented and is still never
touched; the overrides live in a file herd owns outright.

An override **beats the `[mono-focus]` profile**, so any single key the profile
forces can be taken back from the Settings screen without editing the file.

## Session (persisted)

`~/.config/herd/session.json`, holding **only** `config_path` and `model` —
where the program *was*. Anything the user *chose* goes to `Prefs` instead.

## Prefs (persisted)

`~/.herd_config`: a pretty-printed JSON dotfile with sorted keys, meant to be
opened and edited by hand. Holds `favorites`, `overrides` and `router`
(models-max, sleep-idle-seconds), all keyed by preset name rather than tier.

Reading never fails — missing, unreadable or corrupt all mean "no preferences
yet", since losing a convenience must not stop start-up. **Writing does report
failure**, unlike the session file: a silently dropped save loses work done on
purpose. Written to a temporary file and renamed, so an interrupted save cannot
truncate the previous one.

Deliberately *not* persisted: the memory reservation. It is a property of the
machine you are on right now, not a preset setting.

## Shipped preset data (`data/`)

An in-repo snapshot of the user's `~/models/` tiers: `16gb/models.ini`
(14 presets) and `32gb/models.ini` (10 presets), plus the original
`llama-launch.js`, `test_call.sh` and `start-router.sh`.

Reference and **test fixture only** - config resolution reads `~/models/`, never
`data/`. The suite parses these files so every shipped preset is proven to parse
and to build a launchable argv.

## UiEvent

`Key` | `Tick` | `CommandFinished { command, output }` | `Log(String)` |
`LlamaStatus(LlamaSnapshot)` | `PortInUse { port, name, retry }` |
`ChatResult(Box<Result<ChatOutcome, String>>)` | `CacheList(Vec<CachedModel>)` |
`DownloadProgress { model, done, total }` |
`DownloadFinished { model, result }` | `Resize { width, height }` |
`SystemInfo(SystemInfo)` | `AvailableMemory(Option<f64>)` | `Quit`

## Action

`None` | `Quit` | `RunCommand(String)` | `ConfigPathChanged(PathBuf)` |
`RunChat { model, prompt }` | `CopyToClipboard { label, text }` |
`DeleteModel { reference, repo }` |
`Download { model, repo, wants, then_launch }`

`DeleteModel` carries the repo rather than a path: the path is computed inside
`hub::delete_repo`, which is where the fence around it lives, and a path
travelling through the UI is a path something could rewrite on the way.

## Command (`commands.rs`)

`name` (the first token dispatch matches on), `usage` (with its arguments, as
the listing shows it), `summary`, `group` (`Server` | `Config` | `Other`),
`handler` (`App` | `Launcher` | `Script` — which of the three dispatch paths
runs it), `hidden` (`launch!` only) and `probe` (a form the conformance tests
can safely drive).
