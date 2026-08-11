# Roadmap

Status as of 2026-08-11.

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

## Next

Tracked in `doc/PROMPT_NEXT_STEPS.md` (French). Highlights: auto-restart on crash,
a full end-to-end test against a real `llama-server` (spawn, model load, kill),
sizing from the cached GGUF rather than the name, flagging presets duplicated
across tiers, reading vision support from the repo listing rather than the model
name, a `QUANT` column, and router mode as a first-class screen action rather
than a typed command.

## Not started

- Plugin system (see `plugins.md`)
- Device control and automation, the original pre-pivot scope (see `doc/prompt.md`)
