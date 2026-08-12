# Roadmap

Status as of 2026-08-12 (herd 0.7.0).

## Delivered

- Interactive Models screen: preset table, filter, tier switching, live argv preview
- Server screen: OFF / STARTING / SERVING / STOPPING / ERROR, endpoint, uptime, output tail
- Settings screen: session overrides on `[server]`, `[*]` and per-model keys
- `/health` polling as the STARTING -> SERVING signal
- Port-conflict detection with confirm-before-launch
- RAM-tier config resolution, `--config` flag, remembered tier and preset
- Render tests against a headless backend, including a 20x6 terminal
- Process-supervision tests: a live child never blocks the lock, a healthy
  child reaches SERVING end to end, a child that exits on its own is reported
- Opt-in `live` tests against a real running server (`--ignored`)
- Test screen: chat probe (the `test_call.sh` equivalent) with editable prompt,
  latency, token counts and token rate
- Stats screen: session start time, uptime, token counts, throughput
- Preset sizing with red warnings for models the machine cannot hold
- `models.ini` picker (`c`) listing every config and its oversized preset count
- Session-only override of the reserved memory share, with a standing caution
- Local availability: a `LOCAL` column driven by `llama-server --cache-list`,
  confirm-before-download on launch, `d` to fetch without launching, and a
  byte-accurate progress bar
- `OPT` and `CAPS` columns, and a table that sizes itself to the terminal rather
  than clipping off the right edge
- Graceful exit: `q` names what would be abandoned and asks, `Q` forces, and
  shutdown stops the downloader as well as the server — every step bounded
- Build stamping (`herd --version`, commit + date) and `make release`
- Viewport-aware paging, arrow navigation between screens, list position
  indicators, a logs scrollbar, and booleans that toggle on Enter
- Idle cost measured and cut: no redraw on a tick when no clock is on screen
- Favourites (`★`, kept across restarts) and `~/.herd_config` for everything the
  user chose: favourites, overrides, the router numbers
- `y` copies the exact launch command, quoted onto one line, checked against a
  real shell
- Router mode as a screen: its two numbers on screen with a live argv preview,
  rather than flags passed where nobody could see them
- Screen footers that fit the pane they are drawn in, dropping hints visibly
- **Hub screen**: the cache listed with per-model size, per-repo disk, and which
  preset in this tier names it — the ones none do in cyan, `y` copying a stanza
  that would adopt one
- **Measured sizes**: a downloaded preset is sized from its weights file rather
  than from its name, and the table marks which is which
- **Time to first token** on the Stats screen, beside throughput
- A scrollbar on the Models and Hub lists when they overflow
- `:help` as an overlay listing every command, from the same table the
  dispatchers are checked against
- A version number that moves at least a patch level on every commit
  (`hooks/pre-commit`, installed by `make hooks`)
- Deleting a cached model (`D` on the Hub screen) behind a prompt that states
  the size and what else in the repo goes with it
- `:about` — the build stamp and the machine's own facts (config, tier, memory,
  cache) in one place, rather than spread across four screens

## Next

Tracked in `doc/PROMPT_NEXT_STEPS.md` (French). Highlights: auto-restart on
crash, a full end-to-end test against a real `llama-server` (spawn, model load,
kill), flagging presets duplicated across tiers, reading vision support from the
repo listing rather than the model name, a `QUANT` column, sizing a preset that
is *not* yet downloaded from the HuggingFace tree API, and covering router mode
with the port-in-use prompt.

## Retired

The project began as a multi-device console (Flipper Zero, iPhone, Switch) and
pivoted to llama-server. Everything left of that scope has now been removed
rather than carried as intent nobody was going to act on:

- the `scan` and `test` commands, which answered fixed strings
- `services/network.rs`, which existed only for `scan`
- the plugin system, which was a proposal with no trait, registry or loader —
  extension happens by adding an entry to `commands.rs` and a dispatch arm,
  which `architecture.md` documents
- the generation-era prompts and the original méta-prompt

`sh <command>` is the one piece kept: it does what it claims. The provenance
is in git history, which is where a record of what a project used to be
belongs — a spec that describes something the code does not do is worse than
no spec.
