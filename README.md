# HERD

A terminal launcher for local LLMs. Browse the model presets in your `models.ini`, launch one, watch it come up, and tune its options — without typing a `llama-server` command line.

It is a Rust port and expansion of the `llama-launch.js` idea: that script resolves `models.ini` precedence and *prints* an argv. HERD resolves the same precedence, shows you the argv live, then actually spawns and supervises the process.

## Screens

| | Screen | What it does |
|---|---|---|
| `1` | **Models** | The presets in the active `models.ini`, with repo, context size, memory size, optimisations, capabilities, speculative head and whether the weights are on this machine. Launch with `Enter`, download with `d`, star with `f`, copy the launch command with `y`. The active preset is marked `●` serving, `◐` starting or stopping, `✖` failed. |
| `2` | **Hub** | What llama.cpp actually has in its cache: every model, its size, what its repo costs on disk, and which preset in this tier uses it. Models no preset names are in **cyan**; `y` copies a `models.ini` stanza for one. |
| `3` | **Server** | Lifecycle state, model, endpoint, uptime, and the tail of the process output. |
| `4` | **Router** | llama-server's built-in multi-model mode: how many models stay resident, how long an idle one survives, and the argv that starts it. |
| `5` | **Test** | Send a chat completion to the running model and see the reply, when it was sent, the latency, token rate, and llama.cpp's own split of prompt-eval vs generation. |
| `6` | **Stats** | Session counters — start time, uptime, tokens in/out, throughput, time to first token — and the memory budget. |
| `7` | **Settings** | Every `[server]`, `[*]` and per-model key, overridable — kept in `~/.herd_config`. |
| `8` | **Logs** | Full log history, scrollable, with a position indicator and a scrollbar. |

```
┌HERD 0.5.0────────────┐┌ Models · 32gb · ~/models/32gb/models.ini ────────────────────── 1/8 ┐
│ ▸ 1 Models           ││   NAME            REPO              RAM     OPT CAPS SPEC   LOCAL   │
│   2 Hub              ││▸★●gemma4-12b      unsloth/gemma-4…  7.3G  qat ud    S  mtp         █│
│   3 Server           ││  ★gemma4-31b      unsloth/gemma-4…~18.3G  qat ud    S  mtp not local║
│   4 Router           ││   qwen3-coder     unsloth/Qwen3-C… 17.5G  ud moe    C    -         ║│
│   5 Test             ││                                                                     │
│   6 Stats            ││  RAM 36 GiB · quantisation-aware training · speculative (mtp)       │
│   7 Settings         ││  j/↓ move · enter launch · s stop · d download · / filter · …      │
│   8 Logs             │└─────────────────────────────────────────────────────────────────────┘
│ tier  32gb           │┌ argv preview ─────────────────────────────────────────────── y copy ┐
│ RAM   36 GiB         ││llama-server \                                                       │
│                      ││  --host 0.0.0.0 --port 1234 --jinja --ctx-size 32768 \              │
│                      ││  --gpu-layers 99 --hf-repo unsloth/gemma-4-12B-it-qat-GGUF…         │
└──────────────────────┘└─────────────────────────────────────────────────────────────────────┘
 SERVING  gemma4-12b · up 04:12 · http://127.0.0.1:1234 · tab screen · : command · q quit
```

The table **sizes itself to the terminal**. On a narrow one the least
load-bearing columns are dropped in order — context size, then optimisations,
then capabilities, then the speculative head — rather than being clipped off the
right edge where you cannot tell an empty column from a cut-off one. The preset
name and `LOCAL` are never dropped.

A **scrollbar** appears on the right border when there are more presets than
rows on screen, and only then: a full-height thumb beside a list that fits would
imply presets below the fold that do not exist. The `1/8` on the border says
where the cursor ended up; the bar says how much list is above and below it
while you hold a page key.

The **key hints do the same thing**. A footer too long for its pane does not
wrap — its last hints simply disappear off the right edge — so hints are dropped
from the end, least useful first, and the line ends in `…` to say that some
were. `?` always has all of them.

**`f` stars a preset**, `★` in gold. Starring never reorders the table: a list
you navigate by position must not rearrange itself because you starred something
above. Stars are kept in `~/.herd_config`.

**`y` copies that argv as a shell command**, quoted onto one line so it can be
pasted straight into a terminal, a script or a bug report — the same argv HERD
would spawn, session overrides included, rather than a re-typing of the pane.
Alt-screen text does not select cleanly with the mouse, so a preview you can
only read is half an answer to "what would this actually run?". The log line
appears once the clipboard has actually taken it; if no clipboard tool answered
(`pbcopy` on macOS, `wl-copy`/`xclip`/`xsel` elsewhere) it says that instead.

## Run

```bash
cargo run
cargo run -- --config ~/models/16gb/models.ini   # pick a specific preset file
cargo run -- --help
cargo run -- --version                           # herd 0.1.0 (a1b2c3d 2026-08-11)
```

`--version` reports the commit the binary was built from, with `-dirty` when it
came from a tree with uncommitted changes. A version number alone cannot tell
two builds between releases apart; the commit can.

## Keybindings

Global:

| Key | Action |
|-----|--------|
| `1`–`8` | jump to a screen |
| `c` | choose which `models.ini` to use |
| `Tab` / `Shift-Tab`, or `→` / `←` | cycle screens |
| `:` | command bar (power-user escape hatch) |
| `?` | key reference |
| `q` | quit — asks first if a download, probe or command is in flight |
| `Q` | quit at once, abandoning it |

Models screen:

| Key | Action |
|-----|--------|
| `Up`/`Down` or `j`/`k` | move the cursor |
| `PgUp`/`PgDn` | move by a screenful — sized to your terminal, not a fixed 10 |
| `g`/`Home`, `G`/`End` | jump to the first / last row |
| `Enter` | launch the highlighted preset |
| `d` | download it without launching |
| `f` | star it, or take the star off |
| `y` | copy the launch command to the clipboard, quoted and ready to paste |
| `s` | stop the running server, or clear a failed launch |
| `/` | filter by name or repo (`Enter` keeps it, `Esc` clears it) |
| `t` / `T` | next / previous RAM tier |
| `c` | open the config picker |
| `r` | reload `models.ini` from disk |

Hub screen: `j`/`k` move, `y` copies a `models.ini` stanza for the highlighted
model, `Enter` shows its preset on the Models screen, `r` asks llama.cpp again
what it has. There is deliberately **no delete key** — see
[The Hub screen](#the-hub-screen).

Router screen: `j`/`k` move between the two settings, `+`/`-` adjust them,
`Enter` starts the router with what is on screen, `s` stops it, `r` resets both
to their defaults, `y` copies the command. See [Router mode](#router-mode).

Server screen: `s` stop, `p` ping, `Enter` launch the selected preset — refused
when that preset is the one already serving, since relaunching it is a stop and
a full reload for no gain. The screen says so in an `enter` field rather than
leaving you to press the key and read the log. (The Models screen deliberately
still allows it: pressing `Enter` there after changing a setting is how a
session override gets applied.)

Test screen: `Enter` send, `e` edit the prompt, `r` reset it.

Stats screen: `+` / `-` adjust the memory reservation, `r` reset it.

Settings screen: `Up`/`Down` move, `Enter` edit — or **flip** it, when the value
is `true`/`false`, `on`/`off` or `yes`/`no` — `x` clear one override, `X` clear
all. Toggleable rows carry a `[x]`/`[ ]` checkbox, so a row that flips looks
different before you press anything.

Logs screen: `k`/`j` scroll, `PgUp`/`PgDn` by a page, `g` oldest, `G` back to
the newest line. A scrollbar on the right border shows where you are; nothing is
drawn when the whole buffer fits.

## Is the model actually on this machine?

The `LOCAL` column says so, and it is answered by `llama-server --cache-list`
rather than by looking at the filesystem — llama.cpp is what has to load the
file, so its opinion is the one that counts. It is also better informed: it
correctly refuses to list a repo whose current revision is only half
downloaded, which no directory listing would catch.

Until llama.cpp has been asked, nothing is claimed either way. Telling you to
download a model you already have is the one mistake this can make, so it stays
quiet rather than guess.

Pressing `Enter` on a preset that is not here asks first, naming the size:

```
┌ Not downloaded ──────────────────────────────────────────────────┐
│  The weights for this preset are not on this machine.            │
│  It would be fetched from unsloth/gemma-4-31B-it-qat-GGUF.       │
│  Several gigabytes over your connection, then it launches.       │
│                                                                  │
│  Launch 'gemma4-31b' anyway?   [y] yes   [any other key] cancel  │
└──────────────────────────────────────────────────────────────────┘
```

Say yes and the argv preview is replaced by a progress bar until it finishes,
then the model launches on its own. `d` does the download without launching,
which is what makes a mostly-empty tier usable — you can queue up the ones you
know you will want instead of discovering each one at the moment you need it.

Which files get fetched comes from the preset, not from assumption: the weights
matching its quantisation tag, the vision projector unless it says `no-mmproj`,
and the MTP head only when its `spec-type` uses one. Files are named outright
rather than globbed, so a repo holding a dozen quantisations cannot surprise you
with the wrong twenty gigabytes.

The download itself is delegated to the `hf` CLI, which owns the HuggingFace
cache layout — the part that must not be got wrong. Progress is *measured* from
the bytes landing on disk rather than parsed out of anything either downloader
prints.

## The Hub screen

The Models screen answers "what can this tier launch". The Hub screen answers
the other half: **what is actually on this machine**, which is not the same
list. A tier names presets that were never downloaded, and the cache holds
models no tier names — several gigabytes each, invisible until something runs
out of disk.

```
┌ Hub · llama.cpp model cache ────────────────────────────────────────── 1/11 ┐
│  MODEL                                        SIZE     DISK PRESET          │
│▸ unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL      6.3G    13.1G gemma4-12b      │
│  unsloth/gemma-4-31B-it-qat-GGUF:Q4_K_XL     16.1G    33.8G gemma4-31b      │
│  …6-35B-A3B-Claude-4.7-Opus-abliterated:Q4_K 20.2G    21.1G —               │  ← cyan
│  unsloth/Qwen3-14B-GGUF:Q4_K_XL               8.5G     8.5G qwen-3-14b-inst…│
│                                                                             │
│  11 model(s) · 191.1G on disk · 3 not named by this tier (in cyan)          │
│  j/↓ move · y copy preset · enter show · r refresh                          │
└─────────────────────────────────────────────────────────────────────────────┘
```

`SIZE` and `DISK` are different numbers on purpose. `SIZE` is the weights of
that quantisation in the **current** revision — the file llama.cpp would open.
`DISK` is everything the repo occupies, which includes every revision it has
ever fetched: `gemma-4-12B-it-qat-GGUF` above holds two copies of the same 6.3G
weights, and the 6.8G difference is reclaimable. A `*` on `DISK` means two
cached quantisations share that directory, so the figure is not one model's
alone — said rather than divided, because the cache keeps no per-quantisation
accounting and splitting it would be inventing a number.

**`y` copies a `models.ini` stanza** for the highlighted model:

```ini
[qwen3-14b]
hf-repo = unsloth/Qwen3-14B-GGUF:Q4_K_XL
alias = qwen3-14b
```

Just the two keys that make a preset launchable — `[*]` already carries the
context size, the GPU layers and the rest, and a stanza restating them would
fight the defaults the file is built around. It goes to the clipboard rather
than into the file: `models.ini` is hand-maintained and commented, and HERD does
not write to it.

**There is no delete key.** Freeing 17 GiB is not something to offer one
keystroke away from `j`, and HERD's rule everywhere else is that it does not
touch what it did not put there — the same reason the Stats screen prints a
`sysctl` line instead of running it.

## What a preset is and what it can do

Two compact columns, spelled out in full for the highlighted row in the footer
below the table — which doubles as the legend.

| Column | Reads | Meaning |
|---|---|---|
| `OPT` | `qat` | quantisation-aware training |
| | `ud` | Unsloth dynamic quantisation |
| | `moe` | mixture of experts — the `A3B` in `35B-A3B` is the *active* count, and memory sizes on the total |
| `CAPS` | `V` / `v` | vision — uppercase in use, lowercase available but switched off |
| | `S` / `s` | speculative decoding |
| | `A`, `C` | audio, code |
| `SPEC` | `mtp`, `eagle3`, … | *which* speculative head, where `S` only says whether |

All of it is read from the repo reference and the preset's own keys. Nothing
comes from vendor knowledge that would rot — "Qwen3 supports thinking" is true
today and a lie the moment a Qwen4 lands.

Two deliberate omissions worth knowing:

- **Thinking is not a column.** `reasoning = off` is set on every preset in both
  shipped tiers, so a column driven by it would read the same on every row. It
  is a setting, and the Settings screen is where settings live.
- **Vision under-claims.** `no-mmproj = true` looks like evidence of a
  projector — you would only disable one you had — but it is set defensively,
  including on text-only models. It now only decides whether a capability found
  by *name* is switched on. The cost is that `gemma-4` ships a projector without
  saying so in its name and reads as text-only. That is the right direction to
  be wrong in.

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
│sent 14:32:07 · 1.25s · 24 in / 11 out · 41.7 tok/s         │
│prompt eval 0.31s · generation 0.89s · overhead 0.05s       │
│                                                            │
│Bonjour ! Comment puis-je vous aider aujourd'hui ?          │
└────────────────────────────────────────────────────────────┘
```

The send time and the latency are measured here, so they are always present.
The second line is llama.cpp's own split of the round trip and vanishes against
any server that does not report it — a long prompt-eval on a paging machine
looks identical to a slow model until those are broken out. `overhead` is
whatever the round trip cost beyond the server's own accounting.

While a probe is in flight the screen counts up (`waiting for the model… 3.2s`)
rather than sitting on a motionless message.

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
│  first token 0.42s avg  ·  0.39s last  ·  0.31s best       │
└────────────────────────────────────────────────────────────┘
```

The average is computed from totals (all output tokens over all elapsed time),
not by averaging the per-request rates — otherwise a slow first request would
count as much as a fast later one. Figures come from the probes you run on the
Test screen.

**Time to first token** is the other half of "is this model fast", and the two
come apart exactly where it matters: a model paging its weights in can generate
at a perfectly respectable rate and still leave you looking at nothing for four
seconds, which throughput alone reports as a healthy session. `best` here is the
*shortest*, unlike the best rate.

It is derived rather than watched: the probe is deliberately non-streaming — the
same request `test_call.sh` makes, so the two stay comparable — so nothing sees
the first token arrive. What is known is the whole round trip, measured locally,
and llama.cpp's own `predicted_ms` for the generation; the difference is
everything before the first token, which is queueing, prompt ingestion and the
network. A server that sends no `timings` gets `-` and says so, rather than a
zero.

## Sizing and the memory budget

Each preset shows its resident size, and rows the machine cannot hold are drawn
in **red** (amber for a tight fit):

```
  NAME                   REPO                                CTX    RAM  SPEC
▸ gemma4-12b             unsloth/gemma-4-12B-it-qat-GG…    32768   7.3G  mtp
  gemma4-31b             unsloth/gemma-4-31B-it-qat-GG…    32768 ~18.3G  mtp   ← red on a 16 GiB machine
  qwen36-35b             unsloth/Qwen3.6-35B-A3B-MTP-G…    32768  22.3G  mtp   ← red
```

**A `~` means the number is arithmetic; no `~` means it was measured.** Once the
weights are in the cache there is a real file to read, so HERD reads it: the
gguf the current revision would load, plus the same runtime allowance the
estimate adds, so a measured row and an estimated one mean the same thing and
can be judged against the same budget. Only a quantisation that is genuinely
cached counts — a repo you have in Q4 tells you nothing about the size of its
Q8, and a confident wrong number is worse than an honest estimate.

The estimate, for everything not yet downloaded, reads the parameter count and
quantisation out of the repo name — the only sizing information a `models.ini`
carries — and adds the same fixed runtime allowance. A mixture-of-experts name
like `35B-A3B` counts as 35B, since every expert is resident. **A preset whose
name carries no parseable size is never flagged**: a wrong red warning is worse
than none.

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

Edits made on the Settings screen apply to the next launch and are **remembered
in `~/.herd_config`**, so a preset tuned once stays tuned. `models.ini` is still
never written to: those files are hand-maintained and heavily commented, and no
ini round-tripper preserves comment placement reliably. That rule was always
about the ini, not about forgetting — so the overrides live in a file HERD owns,
and the ini stays the untouched thing they are shown against.

An override is exactly a CLI override, so it slots into the precedence chain you already know:

```
[server] → [*] → [model] → your overrides → explicit CLI args
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

The last tier and the last launched preset are remembered in `~/.config/herd/session.json` so a restart lands where you left off — where the program *was*.

What you *chose* goes to `~/.herd_config`: starred presets, setting overrides,
and the router's two numbers. It is pretty-printed JSON with sorted keys,
sitting in `$HOME` rather than in a nested application directory, and is meant
to be read and edited by hand:

```json
{
  "favorites": ["gemma4-12b", "qwen3-coder"],
  "overrides": {
    "global": {},
    "per_model": { "gemma4-12b": { "ctx-size": "65536" } }
  },
  "router": { "models_max": 2, "sleep_idle_seconds": 300 }
}
```

Favourites and overrides are keyed by preset name rather than by tier, since
`gemma4-12b` appears in both shipped tiers and is the same model. A missing or
corrupt file reads as "no preferences yet" and never stops HERD from starting; a
save that fails says so on stderr, because losing something you set on purpose
is not the same as forgetting which tier you were on.

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
│  herd did not start that process and will not stop it.   │
│                                                          │
│  Launch 'gemma4-12b' anyway?  [y] yes  [any key] cancel  │
└──────────────────────────────────────────────────────────┘
```

It never kills a process it did not spawn. When the process on that port *is* herd's own, launching hot-swaps it cleanly with no prompt.

## Router mode

The Models screen supervises one preset at a time. **Router mode is the other
trade**: one long-lived `llama-server` pointed at the whole `models.ini`, loading
and unloading presets on demand. Screen `3` is where its two numbers live —
until now they were flags on a `:router` command line, where nobody could see
them.

```
┌ Router · 32gb ──────────────────────────────────────────────────── 1/2 ┐
│  state     not running                                                 │
│  presets   ~/models/32gb/models.ini                                    │
│  endpoint  -                                                           │
│                                                                        │
│▸ models-max                2   models kept resident before one is unloaded
│  sleep-idle-seconds     300s   idle time before a model is unloaded    │
│                                                                        │
│  j/↓ move · +/- adjust · enter start · s stop · r reset                │
└────────────────────────────────────────────────────────────────────────┘
┌ argv preview ─────────────────────────────────────────────── y copy ┐
│llama-server \                                                       │
│  --host 0.0.0.0 --port 1234 --jinja --models-preset ~/models/32gb/… │
│  --models-max 2 --sleep-idle-seconds 300                            │
└─────────────────────────────────────────────────────────────────────┘
```

`Enter` starts it with exactly what is on screen; the argv preview below is the
same live view the Models screen carries, because the two numbers only mean
anything as the flags they become. Both are remembered in `~/.herd_config`.

The state line reports the supervised process **only when it is the router**. A
single preset serving from the Models screen is not this screen's business, and
showing SERVING for it would claim the router was up when it is not.

`[server]` is shared: the router binds the same host and port your presets do,
so it and a manually launched preset cannot run at once.

## Commands

The `:` bar remains available for anything faster to type than to navigate:

- `models` / `reload` -> re-read `models.ini`
- `launch <model> [-- extra args]` -> launch a preset, hot-swapping any running one
- `router [--max N] [--idle S]` -> start llama-server's native router, which loads and unloads models on demand (defaults: `--max 2 --idle 300`)
- `stop`, `status`, `ping <model>`
- `cache` -> ask llama.cpp again what it has, after a download or a deletion made outside HERD (also `r` on the Hub screen)
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

## Quitting

`q` quits outright when nothing is in flight. When something is, it names what
would be abandoned and asks:

```
┌ Work in progress ────────────────────────────────────────────────┐
│  Quitting now would abandon:                                     │
│    · downloading gemma4-31b (2.1G of 6.7G · 31%)                 │
│                                                                  │
│  The server is stopped on exit either way.                       │
│  Quit anyway?   [y] yes   [any other key] stay                   │
└──────────────────────────────────────────────────────────────────┘
```

`Q` skips the question. A *running server* is deliberately not a reason to ask —
it is stopped on exit every time by design, and prompting on the normal case is
how you train someone to dismiss prompts unread.

Shutdown stops the downloader as well as the server, and every step is bounded:
SIGTERM, then SIGKILL, then giving up. A kill that takes twenty seconds cannot
make the app hang on exit.

## Building a release

```bash
make release          # verify, bump the patch version, tag, build
make release-minor    # or -major
VERSION=1.0.0 make release
```

It refuses on a dirty tree — a release has to be reproducible from its tag. The
version is **not** auto-incremented on every release build: a build script
rewriting `Cargo.toml` would dirty the tree on each `cargo build --release`,
invalidate its own fingerprint and rebuild in a loop, and produce numbers that
count builds rather than releases. Individual builds are told apart by the
commit stamp instead.

## Behavior notes

- Logs are capped at 500 entries; multi-line output is split into separate entries before the cap is applied.
- `sh <command>` is bounded by a 30s timeout; failures show `exit <code>: <stderr>` or `timeout after 30s`.
- If a command task panics or is cancelled, the UI still receives a `task aborted` completion so the prompt never gets stuck.
- Rendering is a pure function of the app state and is covered by tests against a headless backend, including a 20x6 terminal.
- A batch of events costs one frame, not one frame each: a loading `llama-server` emits its output in bursts of hundreds of lines.
- At rest herd does not redraw at all. Only the clocks advance on their own, so a tick with none on screen skips the draw — that is 80ms of CPU per 30s idle down to 30ms.
- Idle footprint is ~10.5 MB RSS, which is why the runtime is left alone: the memory that matters is the model's.
- `STARTING` says which part it is in — binding the port, downloading weights, or loading them — with the elapsed time. Four minutes of a bare "STARTING" is indistinguishable from a hang.
- `/health` is polled for the whole life of a launch, not just until the first success. A server that goes quiet while its process stays alive is marked as not responding rather than left reading `SERVING`.
- A failed launch is diagnosed rather than reported raw: `SIGKILL` reads "killed by the system — most likely out of memory", with the preset's estimate and your budget alongside when the size was already a concern.
