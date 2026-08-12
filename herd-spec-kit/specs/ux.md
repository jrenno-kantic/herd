# UX Specification

## Philosophy

Inspired by lazygit and k9s: dense, keyboard-first, one screen per concern.

## Screens

| | Screen | Purpose |
|---|---|---|
| `1` | Models | Preset table for the active `models.ini` + live argv preview |
| `2` | Server | Lifecycle state, endpoint, uptime, recent output |
| `3` | Router | llama-server's own multi-model mode + the argv it would start |
| `4` | Test | Chat probe against the running model: reply, latency, token rate |
| `5` | Stats | Session counters, TTFT (cold, last, average), and the memory budget |
| `6` | Settings | Editable `[server]` / `[*]` / per-model keys |
| `7` | Logs | Full history |
| `8` | Hub | What llama.cpp has in its cache: size, disk, and the preset using it |

Menu order follows **how often a screen is wanted**, not how closely it relates
to its neighbour: the first seven are what a session moves through, and Hub is
where you go occasionally to see what the cache is holding.

The digits are **positional**: inserting or moving a screen renumbers the rest.
Nothing may hard-code one — not a test, and not a string in a component. The
order lives in `Screen::ALL` alone, so moving one is a single line.

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
- `ConfirmDelete` - answering "this removes a cached model". Its own mode
  rather than another launch prompt: every other confirmation asks whether to
  *start* something, and answering the wrong one of those costs a launch. This
  one costs the download
- `Help` - the `?` reference card for keys; any key dismisses it
- `Commands` - the `:help` listing of what the command bar accepts
- `About` - the `:about` dialog: which build this is, and what it is running
  against

`Help`, `Commands` and `About` are three overlays rather than one because they
answer three different questions — "what does this key do", "what can I type",
"what am I running" — and a single list of all three would bury each in the
others. All three close on any key, and all three are answered locally, before
the busy gate: they are what a stuck user reaches for.

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
its preset on the Models screen, `r` asks llama.cpp again what it has, and `D`
deletes it from the cache.

`D` is the only destructive key in the program, and is fenced accordingly:
uppercase because lowercase `d` next door *downloads*; a prompt that states the
size and names any other quantisation sharing the directory before asking; only
a lowercase `y` accepts, unlike the launch prompts; and an outright refusal —
not a warning — for the repo a live server is serving from or a download is
still writing into.

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
`true`/`false`, `on`/`off` or `yes`/`no` — `m` switch the `[mono-focus]` profile
for this preset, `x` clear one override, `X` clear all.

The profile's keys are listed **only while it is on**, and the heading carries
the state: rows that look editable and are not in force are worse than no rows,
and an absent section and a switched-off one look identical otherwise.
Toggleable rows carry a `[x]`/`[ ]` checkbox so they look different before the
key is pressed.

Logs: `k`/`j` scroll, `PgUp`/`PgDn` by a page, `g` oldest, `G` back to newest.
A scrollbar on the right border shows the position, and nothing is drawn when
the whole buffer fits. The Models, Hub and Settings lists carry the same bar
under the same condition; on Settings it is counted in rows rather than entries,
because a section header takes two rows for one item.

## Feedback

- Status bar leads with the lifecycle tag, coloured: green SERVING, yellow
  STARTING/STOPPING, red ERROR, dim OFF
- Endpoint and uptime appear as soon as a launch starts
- The argv preview updates as the cursor moves, so the exact command is visible
  before committing to it. It wraps to the pane rather than running off the
  right edge, and scrolls (`J`/`K`, with a scrollbar) when the command is taller
  than the pane — a preview that showed only part of a command, silently, would
  be worse than none
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
- **Anything that needs a running server says so when there is none.**
  `:status`, `:ping` (and `p` on the Server screen) and the Test screen's probe
  report "nothing is listening on <endpoint> — no llama-server is running", with
  the two ways to start one, rather than the HTTP client's account of the
  plumbing. A timeout stays a timeout: something answered the door, which is a
  different problem. They attempt first rather than refusing on herd's own
  state, since a server started outside herd is a supported thing to probe
- Config errors render in the Models screen and never abort the UI
- The command bar names what the typed line would run, so a typo reads as
  `unknown` before Enter rather than after it
- Input is ignored while a command is in flight — except `stop` and `:help`,
  the two things wanted *because* something is stuck
