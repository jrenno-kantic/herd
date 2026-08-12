# UX Specification

## Philosophy

Inspired by lazygit and k9s: dense, keyboard-first, one screen per concern.

## Screens

| | Screen | Purpose |
|---|---|---|
| `1` | Models | Preset table for the active `models.ini` + live argv preview |
| `2` | Hub | What llama.cpp has in its cache: size, disk, and the preset using it |
| `3` | Server | Lifecycle state, endpoint, uptime, recent output |
| `4` | Router | llama-server's own multi-model mode + the argv it would start |
| `5` | Test | Chat probe against the running model: reply, latency, token rate |
| `6` | Stats | Session counters, time to first token, and the memory budget |
| `7` | Settings | Editable `[server]` / `[*]` / per-model keys |
| `8` | Logs | Full history |

The digits are **positional**: inserting a screen renumbers the ones after it.
Nothing may hard-code one — not a test, and not a string in a component. Both
`Hub` and `Router` were inserted next to the screen they answer half a question
with, and the renumbering cost nothing but this rule.

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
- `ConfirmLaunch` - answering a launch prompt: port in use, too large, or not
  downloaded
- `ConfirmQuit` - answering "this would abandon work in flight"
- `Help` - the `?` reference card for keys; any key dismisses it
- `Commands` - the `:help` listing of what the command bar accepts. Separate
  from `Help` because the two answer different questions — "what does this key
  do" against "what can I type" — and one list of both would bury each in the
  other

## Keybindings

Global:

| Key | Action |
|-----|--------|
| `1`-`8` | jump to a screen |
| `c` | choose which `models.ini` to use |
| `Tab` / `Shift-Tab` or `→` / `←` | cycle screens |
| `:` | command bar — `:help` lists what it accepts |
| `?` | key reference |
| `q` | quit — asks first if work is in flight |
| `Q` | quit at once, abandoning it |

Models:

| Key | Action |
|-----|--------|
| `Up`/`Down` or `j`/`k` | move the cursor |
| `PgUp`/`PgDn` | move by a screenful, sized to the terminal |
| `g`/`Home`, `G`/`End` | first / last row |
| `Enter` | launch the highlighted preset |
| `d` | download it without launching |
| `f` | star it, or take the star off |
| `y` | copy the launch command to the clipboard |
| `s` | stop the server, or clear a failed launch |
| `/` | filter (`Enter` keeps, `Esc` clears) |
| `t` / `T` | next / previous tier |
| `r` | reload from disk |

Hub: `y` copies a `models.ini` stanza for the highlighted model, `Enter` reveals
its preset on the Models screen, `r` asks llama.cpp again what it has. There is
deliberately **no delete key**: freeing 17 GiB is not something to offer one
keystroke away from `j`, and herd does not touch what it did not put there.

Router: `j`/`k` move between the two settings, `+`/`-` adjust them, `Enter`
starts the router with exactly what is on screen, `s` stops it, `r` resets both,
`y` copies the command. `Enter` is always allowed here, unlike the Server
screen's — the numbers it starts with are the ones on screen, and having just
changed one is the reason to press it.

Server: `s` stop, `p` ping, `Enter` launch selected — refused when the selected
preset is the one already serving, since relaunching it is a stop and a full
reload for no gain. The screen states that in an `enter` field rather than
leaving it to be discovered by pressing the key.

Test: `Enter` send, `e` edit prompt, `r` reset.

Stats: `+` / `-` adjust the memory reservation, `r` reset.

Settings: `Up`/`Down` move, `Enter` edit — or *flip* it, when the value is
`true`/`false`, `on`/`off` or `yes`/`no` — `x` clear one override, `X` clear all.
Toggleable rows carry a `[x]`/`[ ]` checkbox so they look different before the
key is pressed.

Logs: `k`/`j` scroll, `PgUp`/`PgDn` by a page, `g` oldest, `G` back to newest.
A scrollbar on the right border shows the position, and nothing is drawn when
the whole buffer fits.

## Feedback

- Status bar leads with the lifecycle tag, coloured: green SERVING, yellow
  STARTING/STOPPING, red ERROR, dim OFF
- Endpoint and uptime appear as soon as a launch starts
- The argv preview updates as the cursor moves, so the exact command is visible
  before committing to it
- Overridden settings are marked `*` and shown next to the ini value they replace
- Presets the machine cannot hold are drawn in red, tight fits in amber; a
  preset whose size cannot be read is never flagged
- A size the weights were *measured* at is printed plainly; one derived from the
  repo name carries a `~`, because the heuristic has been wrong by a factor of
  four and the two are not equally certain
- A starred preset carries `★` in gold — a glyph, not merely a colour, so it
  survives a screenshot, a colour-blind reader and a terminal with its own
  palette. Starring never reorders the table
- A cached model no preset in this tier names is drawn in cyan and counted in
  the Hub summary. Not red: it is not an error, and it may belong to another tier
- Both key hints and the command listing are dropped or elided *visibly* when
  the pane is too narrow — a hint that vanished silently reads as a key that
  does not exist
- Presets that are not on the machine say so in a `LOCAL` column; until
  llama.cpp has been asked, nothing is claimed either way
- Launching one asks first, naming the download size, then shows a byte-accurate
  progress bar and launches on its own when it finishes
- STARTING says which part it is in — binding, downloading, or loading weights —
  with the elapsed time, because four minutes of a bare "STARTING" is
  indistinguishable from a hang
- A server that stops answering while its process is alive is marked as not
  responding rather than left reading SERVING
- Lowering the memory reservation below the system default raises a standing red
  caution — the override is allowed, never silent
- Config errors render in the Models screen and never abort the UI
- The command bar names what the typed line would run, so a typo reads as
  `unknown` before Enter rather than after it
- Input is ignored while a command is in flight — except `stop` and `:help`,
  the two things wanted *because* something is stuck
