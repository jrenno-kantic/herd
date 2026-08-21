# Roadmap

Status as of 2026-08-21 (herd 0.8.9; stable Homebrew formula 0.8.7).

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
- The Models `REPO` column consumes every spare cell while fixed columns remain
  visible
- Graceful exit: `q` confirms while a manual or router process is live, **and**
  while a download, probe or command is in flight, naming its serving state and
  the work that would be interrupted; with neither it exits directly, `Q`
  forces, and shutdown remains bounded
- Sidebar machine telemetry: architecture, GPU, installed RAM and available
  memory, refreshed around llama lifecycle changes
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
- `:about` — the build stamp, machine facts and probed external-tool versions
  (config, tier, memory, cache, llama-server, hf) in one place
- Startup preflight: `llama-server` is required before the TUI opens; a missing
  `hf` leaves local launching available while downloads are disabled explicitly.
  Both probes run concurrently under a fifteen-second bound, which only ever
  waits on a binary that is present and slow
- Public Homebrew distribution through `jrenno-kantic/tap/herd-llm`, with
  `llama.cpp` and `hf` declared as runtime dependencies and tap-native CI
- dist 0.32.0 release automation, validated by the `v0.8.8-rc.2` three-platform
  prerelease without displacing the stable Homebrew formula
- A `[mono-focus]` profile: a reserved ini section switched on per preset (`m`
  on Settings) for one client looping on the same base prompt
- **Fixed:** the session overrides reached the argv *preview* and not the
  launch. Both go through `LaunchSettings::argv` now
- An argv preview that wraps to its pane and scrolls (`J`/`K`, with a
  scrollbar), instead of clipping a long command at the right edge and the
  bottom with nothing to say it had
- The Settings list scrolls with a bar of its own, counted in rows so that
  two-row section headers do not put the thumb out, and its rows are clipped
  with a mark rather than cut by the terminal
- **Pointing OpenCode at a preset**: `o` on the Models screen shows the
  `opencode.json` provider block for the highlighted preset and `y` copies it.
  Built from the launch argv, so the endpoint, alias and context size are the
  ones a launch would really use; fields herd cannot support are omitted rather
  than guessed; and the file itself is never written, like `models.ini`
- **Fixed:** `q` quit outright while a download was running. The prompt asked
  about the server alone, while `in_flight` already knew what was at stake. It
  now asks for either, and says that an interrupted download resumes
- The startup preflight bound is fifteen seconds, not three. Three was enough on
  a warm machine and not on a cold one, and a false timeout either aborts before
  the TUI opens or disables downloads for the session

## Next

Resolve the two macOS-only GitHub Verify timeouts in the process-supervision
tests. Local verification and Ubuntu CI pass, but a stable tag must wait for
both CI platforms because the dist Release workflow does not depend on the
separate Verify workflow.

Then publish the first stable dist-managed release and let the tap's Homebrew
CI validate the generated prebuilt `herd-llm` formula before retiring the
working source formula as the active path. The rollback and acceptance checks
remain in `distribution.md`.

`TODO.md` currently has no outstanding tasks. Longer-term candidates
remain in `doc/PROMPT_NEXT_STEPS.md` (French): auto-restart on
crash, a full end-to-end test against a real `llama-server` (spawn, model load,
kill), flagging presets duplicated across tiers, reading vision support from the
repo listing rather than the model name, a `QUANT` column, and sizing a preset
that is *not* yet downloaded from the HuggingFace tree API. (Covering router
mode with the port-in-use prompt shipped: `:router` asks the same question as
`launch`, with `router!` as its hidden force variant, and a hot-swap announces
the stop of the previous server instead of waiting silently.)

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
