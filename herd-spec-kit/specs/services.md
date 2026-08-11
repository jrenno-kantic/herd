# Services

## Llama Service (`services/llama/`)

The product. Five modules:

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
  budget the fit warnings are judged against

Fixtures for this service live in `data/` (a snapshot of `~/models/`); they are
parsed by the tests but never used to resolve config at runtime.

Two launch modes: **manual** (one preset at a time, hot-swapped) and **router**
(one long-lived process that loads and unloads models itself).

## Script Service

Runs predefined commands and owns the `:help` catalogue.

## System Service

Executes shell commands via `sh <command>`, bounded by a 30s timeout.

## Network Service

Stub. `scan_devices` sleeps and returns "No devices discovered".
