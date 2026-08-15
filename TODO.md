## TODO

- [x] Fix router mode which hang to start if a model is already serving
  - Three things stacked into that hang, and each got its own fix:
    - **The hot-swap stop was silent.** `:router` (and `launch`) over a
      running model stops it first, and with a big model paging out that
      stop takes seconds to tens of seconds — during which the UI sat
      frozen on SERVING and every keypress answered "busy". `spawn` now
      announces STOPPING (in the mode being started, so the Router screen
      shows it immediately) before the stop, and only when there is
      something to stop — same rule as `stop_announced`, never flash a
      transition that did not happen.
    - **A stop that overran its grace spawned into a corpse's port.** An
      `Abandoned` stop leaves the predecessor still holding the port, so
      the new process was guaranteed llama-server's raw "couldn't bind
      HTTP server socket" a moment later. `swap_refusal` now refuses the
      spawn with the actual cause: "the previous llama-server is still
      shutting down and holds the port — try again in a few seconds".
    - **The router had no port-in-use prompt** (the known gap): a port
      held by a server herd did not start surfaced as that same raw bind
      error one second after STARTING. `:router` now asks the same
      question as `launch`, and a yes re-dispatches `router!` — hidden
      force variant, same spirit as `launch!`. The retry command now
      travels in the event/confirm (`UiEvent::PortInUse { port, name,
      retry }`), authored verbatim by the Executor that refused it, which
      also fixes a confirmed `launch <model> -- extra args` silently
      dropping its extra args on retry.
  - Verified against the real llama-server (b10360) by driving the actual
    TUI through a pty: model serving → `:router` hot-swaps and announces;
    foreign holder on 1234 → the Port-in-use dialog at +1s, and a
    confirmed retry surfaces the real bind error.
  - **Left out:** no automatic retry once the port frees — the refusal
    says "try again", it does not loop; and confirming the router prompt
    still cannot succeed while the foreign server lives (herd never kills
    a process it did not spawn — the dialog says so).
