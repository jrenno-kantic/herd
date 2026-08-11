# HERD

A terminal launcher for local LLMs. Browse the model presets in your `models.ini`, launch one, watch it come up, and tune its options — without typing a `llama-server` command line.

It is a Rust port and expansion of the `llama-launch.js` idea: that script resolves `models.ini` precedence and *prints* an argv. HERD resolves the same precedence, shows you the argv live, then actually spawns and supervises the process.

## Screens

| | Screen | What it does |
|---|---|---|
| `1` | **Models** | The presets in the active `models.ini`, with repo, context size and speculative-decoding mode. Launch with `Enter`. The active preset is marked `●` serving, `◐` starting or stopping, `✖` failed. |
| `2` | **Server** | Lifecycle state, model, endpoint, uptime, and the tail of the process output. |
| `3` | **Test** | Send a chat completion to the running model and see the reply, latency and token rate. |
| `4` | **Stats** | Session counters — start time, uptime, tokens in/out, throughput — and the memory budget. |
| `5` | **Settings** | Every `[server]`, `[*]` and per-model key, editable for the session. |
| `6` | **Logs** | Full log history. |

```
┌HERD───────────────┐┌ Models · 32gb · ~/models/32gb/models.ini ────────────────────┐
│ ▸ 1 Models           ││  NAME              REPO                     CTX    RAM  SPEC │
│   2 Server           ││▸●gemma4-12b        unsloth/gemma-4-12B…   32768   7.7G  mtp  │
│   3 Test             ││  gemma4-31b        unsloth/gemma-4-31B…   32768  18.3G  mtp  │
│   4 Stats            ││  qwen3-coder       unsloth/Qwen3-Coder…   32768  17.8G   -   │
│   5 Settings         ││                                                              │
│   6 Logs             ││RAM 36 GiB   enter launch · s stop · / filter · c config      │
│ tier  32gb           │└──────────────────────────────────────────────────────────────┘
│ RAM   36 GiB         │┌ argv preview ────────────────────────────────────────────────┐
│                      ││llama-server \                                                │
│                      ││  --host 0.0.0.0 --port 1234 --jinja --ctx-size 32768 \        │
│                      ││  --gpu-layers 99 --hf-repo unsloth/gemma-4-12B-it-qat-GGUF…   │
└──────────────────────┘└──────────────────────────────────────────────────────────────┘
 SERVING  gemma4-12b · http://127.0.0.1:1234 · up 252s · tab screen · : command · q quit
```

## Run

```bash
cargo run
cargo run -- --config ~/models/16gb/models.ini   # pick a specific preset file
cargo run -- --help
```

## Keybindings

Global:

| Key | Action |
|-----|--------|
| `1`–`6` | jump to a screen |
| `c` | choose which `models.ini` to use |
| `Tab` / `Shift-Tab` | cycle screens |
| `:` | command bar (power-user escape hatch) |
| `q` | quit |

Models screen:

| Key | Action |
|-----|--------|
| `Up`/`Down` or `j`/`k` | move the cursor |
| `Enter` | launch the highlighted preset |
| `s` | stop the running server |
| `/` | filter by name or repo (`Enter` keeps it, `Esc` clears it) |
| `t` / `T` | next / previous RAM tier |
| `c` | open the config picker |
| `r` | reload `models.ini` from disk |

Server screen: `s` stop, `p` ping, `Enter` launch the selected preset.

Test screen: `Enter` send, `e` edit the prompt, `r` reset it.

Stats screen: `+` / `-` adjust the memory reservation, `r` reset it.

Settings screen: `Up`/`Down` move, `Enter` edit, `x` clear one override, `X` clear all.

## Testing the running model

The Test screen is `data/scripts/test_call.sh` as a screen: the same system
prompt, the same default message (`Bonjour`), the same non-streaming request —
but the prompt is editable and the result is measured instead of dumped as raw
JSON.

```
┌ Test ──────────────────────────────────────────────────────┐
│  model     gemma4-12b                                      │
│  state     SERVING                                         │
│                                                            │
│  prompt    Bonjour                                         │
│                                                            │
│  enter send · e edit prompt · r reset                      │
└────────────────────────────────────────────────────────────┘
┌ Response ──────────────────────────────────────────────────┐
│1.25s  ·  24 in / 11 out  ·  41.7 tok/s                     │
│                                                            │
│Bonjour ! Comment puis-je vous aider aujourd'hui ?          │
└────────────────────────────────────────────────────────────┘
```

Latency is measured locally, so it is always shown. Token counts come from the
response's `usage` block and the rate from llama.cpp's `timings` extension; when
a server omits either, the line degrades to what it does report rather than
showing nothing.

The probe targets whatever is loaded, falling back to the highlighted preset so
the screen still works against a server started outside herd. It runs outside
the command queue, so a slow generation never locks you out of stopping the
server. `:ping <model>` remains as the one-line command-bar equivalent.

## Stats

Counters for the current serving session, reset on every launch so they describe
*this* model rather than the whole run:

```
┌ Session ───────────────────────────────────────────────────┐
│  model       gemma4-12b                                    │
│  state       SERVING                                       │
│  started     14:32:07                                      │
│  uptime      12m41s                                        │
│                                                            │
│  requests    2                                             │
│  tokens in   48                                            │
│  tokens out  200                                           │
│  throughput  50.0 tok/s avg  ·  51.0 last  ·  51.0 best    │
└────────────────────────────────────────────────────────────┘
```

The average is computed from totals (all output tokens over all elapsed time),
not by averaging the per-request rates — otherwise a slow first request would
count as much as a fast later one. Figures come from the probes you run on the
Test screen.

## Sizing and the memory budget

Each preset shows an estimated resident size, and rows the machine cannot hold
are drawn in **red** (amber for a tight fit):

```
  NAME                   REPO                                CTX    RAM  SPEC
▸ gemma4-12b             unsloth/gemma-4-12B-it-qat-GG…    32768   7.7G  mtp
  gemma4-31b             unsloth/gemma-4-31B-it-qat-GG…    32768  18.3G  mtp   ← red on a 16 GiB machine
  qwen36-35b             unsloth/Qwen3.6-35B-A3B-MTP-G…    32768  20.6G  mtp   ← red
```

The estimate reads the parameter count and quantisation out of the repo name —
the only sizing information a `models.ini` carries — and adds a fixed runtime
allowance. A mixture-of-experts name like `35B-A3B` counts as 35B, since every
expert is resident. **A preset whose name carries no parseable size is never
flagged**: a wrong red warning is worse than none.

### Choosing a config (`c`)

`c` opens a picker over every `models.ini` on this machine, each with its preset
count and how many of them exceed the current budget:

```
┌ Select models.ini ─────────────────────────────────────────────────┐
│  budget 12.0 GiB usable of 16 GiB installed                        │
│   16gb      12 presets                                             │
│     ~/models/16gb/models.ini                                       │
│ ▸• 32gb       8 presets  ⚠ 5 exceed this machine                   │
│     ~/models/32gb/models.ini                                       │
│                                                                    │
│  enter select · up/down move · esc cancel                          │
└────────────────────────────────────────────────────────────────────┘
```

### Overriding the reservation

The budget is installed RAM minus a reserved share for the OS, 25% by default —
roughly what macOS keeps away from the GPU on unified memory. `+` / `-` on the
Stats screen adjust it between 5% and 60%, which changes which presets are shown
as fitting.

Going below the default raises a standing red caution and logs a warning: you are
telling herd the OS needs less than it normally reserves, and if that is wrong
the machine will swap, stall, or have the server killed under load.

**This changes only herd's own judgement — it does not touch any system
setting.** On macOS the real GPU limit is `sudo sysctl iogpu.wired_limit_mb=<MB>`;
herd prints that for you to run yourself rather than running it for you.

## Server lifecycle

```
OFF ──launch──> STARTING ──/health 200──> SERVING ──stop──> STOPPING ──> OFF
                    │                        │
                    └─── spawn/load fails ───┴── crash ──> ERROR ──> OFF
```

`STARTING → SERVING` is confirmed by polling `GET /health` every 500ms, not by pattern-matching the log output — llama.cpp rewords its startup lines between releases, and a state display you cannot trust is worse than none. `ERROR` exists so a model that OOMs or a missing GGUF is visibly distinct from a clean stop.

**STARTING can legitimately last a while.** A preset whose GGUF is not cached yet downloads it first, so the first launch of a 30B model is minutes, not seconds. Watch the Server screen or the Logs: llama-server reports download and load progress there. If nothing is serving after 10 minutes the state becomes `ERROR` rather than hanging forever.

Quitting always stops the supervised process, so no orphaned server keeps holding GPU memory.

## Settings and overrides

Edits made on the Settings screen apply to the next launch and are **discarded on quit**. `models.ini` is never written to: those files are hand-maintained and heavily commented, and no ini round-tripper preserves comment placement reliably.

An override is exactly a CLI override, so it slots into the precedence chain you already know:

```
[server] → [*] → [model] → session overrides → explicit CLI args
```

Overridden keys are marked `*` on the Settings screen and shown next to the value they replaced. Retyping the original value clears the override rather than pinning an identical one.

## Choosing a models.ini

The preset file is resolved once at startup, in this order:

1. `--config <path>` (also `-c <path>` or `--config=<path>`)
2. `$HERD_LLAMA_CONFIG`
3. the tier you last used, if it still exists
4. the RAM tier auto-detected under `~/models/` — directories named `16gb/`, `32gb/`, … each holding a `models.ini`. HERD reads installed RAM and picks the richest tier that fits, falling back to the smallest one if none does.
5. the flat `~/models/models.ini`

Options 1 and 2 are taken at face value: a path that does not exist is reported as a plain `cannot read ...` error rather than being silently swapped for another file.

The last tier and the last launched preset are remembered in `~/.config/herd/session.json` so a restart lands where you left off. Nothing else is persisted.

### `data/`

The repo carries a snapshot of the preset tiers in `data/`:

```
data/
├── 16gb/models.ini      12 presets, 4B–14B: Qwen 3.5 4B/9B (±MTP), Gemma 4 E4B,
│                        Qwen3 4B, Gemma 3 4B, Phi-4 Mini, Nemotron 3 Nano 4B,
│                        Gemma 4 12B, Qwen3-VL 8B, Qwen3 14B
├── 32gb/models.ini       8 presets, 12B–35B: Gemma 4 12B/26B/31B, Qwen 3.6 27B/35B,
│                        Qwen3 Coder 30B, Qwen3-VL 8B, Qwen3 14B
├── scripts/llama-launch.js
├── scripts/test_call.sh
└── start-router.sh
```

It is **reference and test data, not a runtime config source** — resolution still reads `~/models/`. The test suite parses these files directly, so every shipped preset is checked to parse and to build a launchable argv on every `cargo test`.

Both tiers declare identical `[server]` blocks and so bind the same port, which is why the port-conflict prompt exists. `gemma4-12b`, `qwen3-vl-8b-instruct` and `qwen-3-14b-instruct` appear in both tiers.

## Port conflicts

Tiers commonly share a port, so launching from one while another is serving would fail with an opaque address-in-use error. HERD checks the port first and asks:

```
┌ Port in use ─────────────────────────────────────────────┐
│  Port 1234 is already in use.                            │
│  herd did not start that process and will not stop it.│
│                                                          │
│  Launch 'gemma4-12b' anyway?  [y] yes  [any key] cancel  │
└──────────────────────────────────────────────────────────┘
```

It never kills a process it did not spawn. When the process on that port *is* herd's own, launching hot-swaps it cleanly with no prompt.

## Commands

The `:` bar remains available for anything faster to type than to navigate:

- `models` / `reload` -> re-read `models.ini`
- `launch <model> [-- extra args]` -> launch a preset, hot-swapping any running one
- `router [--max N] [--idle S]` -> start llama-server's native router, which loads and unloads models on demand (defaults: `--max 2 --idle 300`)
- `stop`, `status`, `ping <model>`
- `help`, `test`, `scan`, `sh <command>`

## Test

```bash
cargo test                   # unit, render and process-supervision tests
cargo clippy --all-targets
```

A few checks talk to a real server and are skipped by default. Start a
llama-server on `127.0.0.1:1234`, then:

```bash
cargo test -- --ignored --test-threads=1
```

They verify that `/health` answers, that `/v1/models` is parsed, and that port
detection sees the listener — the three things a mocked test cannot confirm
about the llama-server build actually installed.

## Behavior notes

- Logs are capped at 500 entries; multi-line output is split into separate entries before the cap is applied.
- `sh <command>` is bounded by a 30s timeout; failures show `exit <code>: <stderr>` or `timeout after 30s`.
- If a command task panics or is cancelled, the UI still receives a `task aborted` completion so the prompt never gets stuck.
- Rendering is a pure function of the app state and is covered by tests against a headless backend, including a 20x6 terminal.
