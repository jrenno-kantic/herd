# Interactions

- Instant feedback
- No blocking UI: `App::update` does no I/O; command work runs in Executor
  tasks and hardware probes run on blocking workers started by `main`
- External-tool probes finish before the TUI exists: `llama-server` is a
  startup requirement, while an unavailable `hf` disables only HERD-managed
  download actions. Both run concurrently under a fifteen-second bound, which
  only ever waits on a binary that is present and slow — an absent one fails on
  `spawn`
- Async updates: results arrive as `UiEvent`s on the shared channel
- Long-running output (llama-server logs) streams line by line rather than
  arriving in one block at the end
- A command that panics or is cancelled still reports completion, so the
  prompt never gets stuck
- Events are drained before each draw, so a burst of log lines costs one frame
  rather than one frame each
- A tick that changes nothing does not redraw at all: only the clocks advance on
  their own, and at rest there are none
- Leaving is confirmed whenever something would be interrupted — a live server,
  a download, a chat probe, a running command — and the prompt names it. `Q`
  skips the question
