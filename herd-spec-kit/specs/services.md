# Services

## Llama Service (`services/llama/`)

The product. Nine modules:

- `ini.rs` - parses `models.ini`, builds llama-server argv as a `Vec<String>`
  (never a shell string, so no quoting bugs), resolves which config file to use,
  discovers RAM tiers, and shapes the `[preset]` stanza the Hub screen copies
- `process.rs` - `Supervisor` owns one supervised child, streams its
  stdout/stderr as `UiEvent::Log`, and drives the lifecycle state machine.
  Holds the child behind a mutex that must never be locked across an await
  (see the architecture invariants)
- `api.rs` - `health` (the STARTING -> SERVING probe), `port_in_use`
  (pre-launch TCP check), `chat` (the Test screen / `test_call.sh` probe),
  `test_chat` (`:ping`, reply only), `list_models` (`:status`).
  `/v1/models` is parsed leniently: llama-server has shipped both the OpenAI
  shape (`{"data":[{"id":...}]}`) and an Ollama-flavoured one
  (`{"models":[{"name":...}]}`), and only accepting the first reports "no
  models loaded" against a server that is plainly serving one.
  `unreachable` classifies a failed request rather than passing the client's
  message through: a refused connection is reported as "no llama-server is
  running" with the two ways to start one, a timeout as a timeout, and anything
  else through its full source chain. Refusal is detected both by
  `is_connect()` and by the `io::ErrorKind` underneath, since the first has
  moved between reqwest releases and the second has not
- `overrides.rs` - setting overrides, emitted as argv, persisted by `prefs`
- `prefs.rs` - `~/.herd_config`: favourites, overrides and the router numbers.
  What the user *chose*, as against `session.rs`, which records where the
  program *was*. Reading never fails; writing does report failure
- `session.rs` - remembers only the last tier and preset
- `memory.rs` - preset sizing and the budget the fit warnings are judged
  against. **Measured where the weights are on disk** (`Sizing::Measured`), and
  otherwise a heuristic read off the repo name, marked `~` on screen. An
  unrecognised quantisation reports *nothing* rather than assuming Q4 — the
  fallback that did assume it sized a 1-bit model as a 4-bit one, announcing
  16.1 GiB for a 3.5 GiB file
- `hub.rs` - is a preset actually on this machine, and what would it cost to
  fetch. Two authorities, deliberately: `llama-server --cache-list` answers
  "is it here?" (llama.cpp has to load the file, and it correctly refuses to
  list a repo whose current revision is half-downloaded), and the HuggingFace
  tree API answers "what would it cost?" — the sizes the confirmation prompt is
  asking the user to agree to. Fetching is delegated to the `hf` CLI, which owns
  the hub cache layout; progress is *measured* off the growing partial blob,
  because `hf` reports only a file count through a pipe and llama-server writes
  its own partials under a different suffix.
  It also **measures what is there**: per model, the weights of that
  quantisation in the revision `refs/main` names (resolved through the snapshot
  symlinks — summing the repo's blobs would count every revision it has ever
  fetched), and per repo, the disk the whole directory occupies. Both are read
  once per cache refresh.
  `delete_repo` is the **only destructive operation in the program**, and is
  fenced accordingly: the path is computed by `repo_dir` and never taken from
  user text, and it must sit directly under the hub directory, carry the
  `models--` prefix and be an existing directory before anything is unlinked.
  The whole repo goes, not one quantisation — the cache has no per-quantisation
  accounting, and picking blobs out of a shared directory by hand is how a cache
  gets corrupted
- `caps.rs` - what a preset is optimised for (`qat`, `ud`, `moe`) and what it
  can do (vision, speculative, audio, code), read from the repo reference and
  the preset's own keys. Nothing is inferred from vendor knowledge that would
  rot, and `no-mmproj` is deliberately *not* read as evidence of a projector:
  it is set defensively across the shipped tiers, including on text-only models

Fixtures for this service live in `data/` (a snapshot of `~/models/`); they are
parsed by the tests but never used to resolve config at runtime.

Two launch modes: **manual** (one preset at a time, hot-swapped) and **router**
(one long-lived process that loads and unloads models itself). A hot-swap
announces STOPPING before stopping the previous process — that stop is bounded
but not instant, and doing it silently read as a hang — and a stop that runs
out its grace (`Abandoned`) refuses the spawn by naming the cause, since the
predecessor still holds the port and a spawn would only reap a raw bind error.

Either llama-server *or* `hf` can be the one downloading — a launch fetches its
own weights when they are absent — so anything that watches a download has to
account for both.

## Preflight Service

`services/preflight.rs` executes `llama-server --version` and `hf --version`
concurrently before the TUI starts. A successful probe retains the first
non-empty output line; failure distinguishes not-found, execution failure,
non-zero exit and a three-second timeout. This checks that a tool can actually
run, not merely that a path with its name exists.

The result is `Tools { llama_server: Tool, hf: Tool }`. `llama-server` is the
only required tool and prevents startup when unavailable. `hf` is optional:
its error travels into app state so downloads can refuse with that same reason
and `:about` can report it. Pure UI constructors use an assumed-available value
so their tests never depend on the host's `PATH`; production always injects the
real probe.

## Clipboard Service

`shell_command` (pure: POSIX single-quote form, a deliberately conservative safe
set, and `''` for the empty token) and `copy` (the I/O). A platform command
rather than a crate — `pbcopy`, else `wl-copy` / `xclip` / `xsel` — for the same
reason RAM is read with `sysctl`. Each candidate is tried in turn and the log
line is written **only once one has taken the text**: a "copied" that was really
a missing `xclip` is discovered at the paste, which is too late.

## Script Service

All that is left of the generic path is `sh`. The command *catalogue* moved to
`commands.rs`, which is now the only place a command is written down — the copy
that lived here had drifted from the dispatchers.

`test` and `scan` were removed once `:help` started advertising them: both
answered fixed strings, and `scan` never looked at a network. A stub is harmless
until something promises it. The `help` fallback went too — `:help` is answered
in `App::submit_command`, so that arm could no longer fire — along with a 300 ms
`sleep` that only taxed `:sh`.

## System Service

Executes shell commands via `sh <command>`, bounded by a 30s timeout. It also
owns machine telemetry for the sidebar: a compile-time architecture label,
macOS GPU discovery through `system_profiler`, and reclaimable available memory
from `vm_stat` (Linux uses `MemAvailable`). GPU discovery runs once on a
blocking worker; cheaper memory-only samples run again on llama lifecycle
events. Failures produce unknown values, never block startup and never become
invented hardware claims.
