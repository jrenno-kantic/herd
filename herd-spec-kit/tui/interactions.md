# Interactions

- Instant feedback
- No blocking UI: `App::update` does no I/O, all work runs in Executor tasks
- Async updates: results arrive as `UiEvent`s on the shared channel
- Long-running output (llama-server logs) streams line by line rather than
  arriving in one block at the end
- A command that panics or is cancelled still reports completion, so the
  prompt never gets stuck
- Events are drained before each draw, so a burst of log lines costs one frame
  rather than one frame each
- A tick that changes nothing does not redraw at all: only the clocks advance on
  their own, and at rest there are none
