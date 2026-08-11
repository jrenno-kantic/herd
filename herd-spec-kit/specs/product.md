# Product Specification

## Vision

HERD is a terminal launcher for local LLMs. It turns a `llama-server`
`models.ini` into something you browse and act on, instead of a file you read
before typing a long command line.

It descends from `llama-launch.js`, which resolves ini precedence and prints an
argv. HERD resolves the same precedence, shows the argv live, and then
actually spawns and supervises the process.

## Core Use Cases

- See what is in the active `models.ini`: presets with their repo, context size,
  speculative-decoding mode and estimated memory footprint
- Know before launching whether a preset fits this machine, in red when it does not
- Choose between the `models.ini` files present on the machine, each annotated
  with how many of its presets exceed the memory budget
- Launch a preset and watch it move OFF -> STARTING -> SERVING
- Stop it, and see STOPPING before OFF
- Send a chat probe to the running model and read the reply, latency and token rate
- See what the session has done: start time, uptime, tokens in and out, throughput
- Override a server or model option for this session, without editing the file
- Override the share of memory reserved for the OS, with a standing caution
- Read the process output while it runs

## Design Principles

- User-friendly first: every core action has a single keystroke; the `:` command
  bar stays as a power-user escape hatch, not the primary path
- Trustworthy state: the lifecycle shown is confirmed by `/health`, not guessed
  from log wording
- The user's files are theirs: `models.ini` is never written to
- Destructive acts are the user's call: herd never kills a process it did not
  spawn, and never changes a system setting
- Never warn on a guess: anything that cannot be measured or parsed is reported
  as unknown rather than flagged
- Zero latency: the UI thread never blocks on I/O
- No orphaned processes: quitting always stops the supervised server
