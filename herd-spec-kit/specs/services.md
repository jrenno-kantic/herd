# Services

## Llama Service (`services/llama/`)

The product. Eight modules:

- `ini.rs` - parses `models.ini`, builds llama-server argv as a `Vec<String>`
  (never a shell string, so no quoting bugs), resolves which config file to use
  and discovers RAM tiers
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
  models loaded" against a server that is plainly serving one
- `overrides.rs` - session-only setting overrides, emitted as argv
- `session.rs` - remembers only the last tier and preset
- `memory.rs` - heuristic preset sizing from the repo name, and the memory
  budget the fit warnings are judged against. An unrecognised quantisation
  reports *nothing* rather than assuming Q4 — the fallback that did assume it
  sized a 1-bit model as a 4-bit one, announcing 16.1 GiB for a 3.5 GiB file
- `hub.rs` - is a preset actually on this machine, and what would it cost to
  fetch. Two authorities, deliberately: `llama-server --cache-list` answers
  "is it here?" (llama.cpp has to load the file, and it correctly refuses to
  list a repo whose current revision is half-downloaded), and the HuggingFace
  tree API answers "what would it cost?" — the sizes the confirmation prompt is
  asking the user to agree to. Fetching is delegated to the `hf` CLI, which owns
  the hub cache layout; progress is *measured* off the growing partial blob,
  because `hf` reports only a file count through a pipe and llama-server writes
  its own partials under a different suffix
- `caps.rs` - what a preset is optimised for (`qat`, `ud`, `moe`) and what it
  can do (vision, speculative, audio, code), read from the repo reference and
  the preset's own keys. Nothing is inferred from vendor knowledge that would
  rot, and `no-mmproj` is deliberately *not* read as evidence of a projector:
  it is set defensively across the shipped tiers, including on text-only models

Fixtures for this service live in `data/` (a snapshot of `~/models/`); they are
parsed by the tests but never used to resolve config at runtime.

Two launch modes: **manual** (one preset at a time, hot-swapped) and **router**
(one long-lived process that loads and unloads models itself).

Either llama-server *or* `hf` can be the one downloading — a launch fetches its
own weights when they are absent — so anything that watches a download has to
account for both.

## Script Service

Runs predefined commands and owns the `:help` catalogue.

## System Service

Executes shell commands via `sh <command>`, bounded by a 30s timeout.

## Network Service

Stub. `scan_devices` sleeps and returns "No devices discovered".
