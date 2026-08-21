# Product Specification

## Vision

HERD is a terminal launcher for local LLMs. It turns a `llama-server`
`models.ini` into something you browse and act on, instead of a file you read
before typing a long command line.

It descends from `llama-launch.js`, which resolves ini precedence and prints an
argv. HERD resolves the same precedence, shows the argv live, and then
actually spawns and supervises the process.

It began as something else — a multi-device console — and nothing of that scope
survives in the code or in these specs. The history is in git, which is where a
record of what a project used to be belongs.

## Core Use Cases

- See what is in the active `models.ini`: presets with their repo, context size,
  speculative-decoding mode, optimisations, capabilities and estimated memory
  footprint
- Know before launching whether a preset fits this machine, in red when it does not
- See the Mac architecture, GPU, installed RAM, and current available memory in
  the sidebar while models load and unload
- Choose between the `models.ini` files present on the machine, each annotated
  with how many of its presets exceed the memory budget
- Know which presets are actually downloaded, and fetch the ones that are not —
  with a size to agree to first and a progress bar while it runs
- See the other side of that: what the machine holds that no preset in this tier
  names, what each model costs on disk, and the ini stanza that would adopt one
- Delete one to get the disk back, after a prompt that says what goes with it
- Launch a preset and watch it move OFF -> STARTING -> SERVING
- Stop it, and see STOPPING before OFF
- Run the built-in router instead, with its two numbers on screen rather than
  passed as flags nobody can see
- Send a chat probe to the running model and read the reply, latency and token rate
- See what the session has done: start time, uptime, tokens in and out,
  throughput, and how long the model takes to *start* answering
- Star the presets worth coming back to, and keep them across restarts
- Override a server or model option, kept in `~/.herd_config`
- Override the share of memory reserved for the OS, with a standing caution
- Copy the exact launch command, a models.ini stanza, or an `opencode.json`
  provider block, to the clipboard
- Read the process output while it runs, scrollable, with a position indicator
- Ask what can be typed, what a key does, and which build this is, without
  leaving the screen
- Know at startup that `llama-server` can execute, and see both its version and
  the `hf` CLI version (or failure) in `:about`
- Point an editor at a preset: `o` on the Models screen shows the OpenCode
  provider block for it, built from the argv a launch would really spawn
- Confirm normal quit while a manual model or router process is live, or while
  a download, probe or command is in flight, naming the serving mode/model and
  the work that would be interrupted; exit directly when there is neither

## Design Principles

- User-friendly first: every core action has a single keystroke; the `:` command
  bar stays as a power-user escape hatch, not the primary path
- Trustworthy state: the lifecycle shown is confirmed by `/health`, not guessed
  from log wording
- The user's files are theirs: `models.ini` is never written to, and neither is
  `~/.config/opencode/opencode.json`. Both are hand-maintained, both belong to
  someone else, and no round-tripper preserves their comments and key order —
  so what herd has to offer goes to the clipboard instead
- Destructive acts are the user's call: herd never kills a process it did not
  spawn, and never changes a system setting. The one thing it does delete — a
  cached model, on `D` — states the cost first, takes only a lowercase `y`, and
  refuses outright while that model is serving or downloading
- Never warn on a guess: anything that cannot be measured or parsed is reported
  as unknown rather than flagged
- Prefer a measurement to an estimate, and say which is on screen: once the
  weights are on disk there is a real size to read, and the two are not shown as
  equally certain
- Nothing is documented in two places: the keymap, the command list and the
  columns are data, and the tests check them against the code that runs them
- Zero latency: the UI thread never blocks on I/O
- No orphaned processes: quitting always stops the supervised server, and the
  downloader with it
- Nothing unbounded: every wait on a process has a deadline, so the UI cannot be
  made to hang by a slow kill or a stalled download
- Degrade by capability: `llama-server` is required, while a missing `hf`
  disables HERD-managed download actions with an explicit reason but does not
  make local models unusable
- Say which build this is: every binary carries the commit it came from
