# UX Specification

## Philosophy

Inspired by lazygit and k9s: dense, keyboard-first, one screen per concern.

## Screens

| | Screen | Purpose |
|---|---|---|
| `1` | Models | Preset table for the active `models.ini` + live argv preview |
| `2` | Server | Lifecycle state, endpoint, uptime, recent output |
| `3` | Test | Chat probe against the running model: reply, latency, token rate |
| `4` | Stats | Session counters and the memory budget |
| `5` | Settings | Editable `[server]` / `[*]` / per-model keys |
| `6` | Logs | Full history |

## Modes

What a keystroke means depends on the mode. `Browse` is the default and the only
one where letters are shortcuts; every other mode captures text until `Enter` or
`Esc`.

- `Browse` - navigation and shortcuts
- `Command` - the `:` bar
- `Filter` - the `/` preset filter
- `EditSetting` - typing a new value on the Settings screen
- `EditPrompt` - typing the message to send on the Test screen
- `Picker` - choosing a `models.ini`
- `ConfirmLaunch` - answering the port-in-use prompt

## Keybindings

Global:

| Key | Action |
|-----|--------|
| `1`-`6` | jump to a screen |
| `c` | choose which `models.ini` to use |
| `Tab` / `Shift-Tab` | cycle screens |
| `:` | command bar |
| `q` | quit |

Models:

| Key | Action |
|-----|--------|
| `Up`/`Down` or `j`/`k` | move the cursor |
| `Enter` | launch the highlighted preset |
| `s` | stop |
| `/` | filter (`Enter` keeps, `Esc` clears) |
| `t` / `T` | next / previous tier |
| `r` | reload from disk |

Server: `s` stop, `p` ping, `Enter` launch selected.

Test: `Enter` send, `e` edit prompt, `r` reset.

Stats: `+` / `-` adjust the memory reservation, `r` reset.

Settings: `Up`/`Down` move, `Enter` edit, `x` clear one override, `X` clear all.

## Feedback

- Status bar leads with the lifecycle tag, coloured: green SERVING, yellow
  STARTING/STOPPING, red ERROR, dim OFF
- Endpoint and uptime appear as soon as a launch starts
- The argv preview updates as the cursor moves, so the exact command is visible
  before committing to it
- Overridden settings are marked `*` and shown next to the ini value they replace
- Presets the machine cannot hold are drawn in red, tight fits in amber; a
  preset whose size cannot be read is never flagged
- Lowering the memory reservation below the system default raises a standing red
  caution — the override is allowed, never silent
- Config errors render in the Models screen and never abort the UI
- Input is ignored while a command is in flight
