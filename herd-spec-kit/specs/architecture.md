# Architecture

## Layers

1. UI (ratatui)
2. App State
3. Services
4. Execution Engine

## Pattern

Component-based + event-driven, unidirectional (Elm-like).

## Data Flow

```
Input → UiEvent → App::update → Action → Executor → UiEvent → …
                       ↓
                  state update → render
```

A single tokio mpsc channel carries every `UiEvent`. `App::update` is pure and
synchronous: it does no I/O and returns an `Action` (`None`, `Quit`,
`RunCommand`, `ConfigPathChanged`, `RunChat`). All async work happens in
`Executor` tasks, which feed their results back into the same channel.

The last two `Action`s are structured rather than string-encoded on purpose: a
chat prompt is free text that must not be re-parsed out of a command line, and a
config change has to reach the `Executor` so it resolves presets against the
file the UI is showing.

## Key Modules

- `main.rs` : CLI parsing (`--config`), config resolution, event loop, shutdown
- `app.rs` : all state (`App`, `LauncherState`, `SessionStats`) + the pure
  `update` transition
- `event.rs` : `UiEvent` producers (keyboard on a blocking thread, 250ms tick)
- `engine/executor.rs` : async command dispatch, owns the llama `Supervisor`
- `services/llama/` : `ini` (parser + argv builder + tier discovery), `process`
  (supervisor + state machine), `api` (HTTP client), `overrides` (session-only
  settings), `memory` (preset sizing + budget), `session` (remembered tier)
- `services/` : scripts, system (shell), network
- `components/` : read-only renderers over `App` — one per screen
  (`models`, `server`, `test`, `stats`, `settings`, `logs`), plus `sidebar`,
  `command_bar`, `status`, and the `confirm` / `picker` modals
- `layout.rs`, `theme.rs` : geometry and styles

## Command Dispatch

Three distinct paths, worth knowing before adding a command:

1. llama lifecycle (`launch`, `router`, `stop`, `ping`, `status`) : handled in
   `Executor`, which owns the shared `Supervisor`
2. `models` / `reload` : handled synchronously inside `App::submit_command`
   (a cheap local file read; must not set the `running` flag)
3. everything else : the generic `services::scripts::run_script` path

## Invariants

- **No await while holding the `Supervisor` child mutex.** The child lives in an
  `Arc<Mutex<Option<Child>>>` so `stop()` is the single source of truth for "is
  something supervised". Awaiting `child.wait()` under the guard pins the lock
  for the whole process lifetime and deadlocks everything else that needs it —
  the `/health` poller, `is_running`, `stop`, and therefore shutdown. The exit
  watcher polls `try_wait()` with short locks; `stop()` takes the child out of
  the slot before awaiting it.
- `CompletionGuard` guarantees a `CommandFinished` event even if a task panics
  or is dropped, so `running` can never get stuck
- `Executor::shutdown` runs after the event loop exits, so no orphaned
  llama-server keeps holding GPU memory (`kill_on_drop` is the second net).
  This invariant **depends on the lock rule above**: a blocked `stop()` makes
  shutdown hang and leaks the very process it was meant to reap.
- Rendering is a pure function of `App`: no mutation during draw


## Screens and modes

`app.rs` owns a `Screen` (which view is up) and a `Mode` (what a keystroke
means). Only `Browse` treats letters as shortcuts; `Command`, `Filter`,
`EditSetting` and `ConfirmLaunch` capture text until `Enter` or `Esc`. New
shortcuts belong inside a mode, never globally, or they shadow text entry.

`tui.rs::render(frame, app)` is split out of `TerminalSession::draw` so the
whole UI can be rendered against a headless `TestBackend` in tests.

## Keeping App and Executor in step

The active `models.ini` is resolved once in `main.rs` and handed to both. When
the user switches tier, `App::update` returns `Action::ConfigPathChanged` and
`main.rs` forwards it to `Executor::set_config_path` (an `Arc<RwLock<PathBuf>>`
behind the scenes). Closing that loop matters: an Executor still pointing at the
previous tier resolves presets against the wrong file.
