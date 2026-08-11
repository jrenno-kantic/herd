## TODO

- [x] rename ops-tui to a more appropriate name → **herd** (a herd of llamas).
      `$OPS_TUI_LLAMA_CONFIG` and `~/.config/ops-tui/session.json` are still
      read as fallbacks, so the rename costs no remembered state.
- [x] add git local repo → the outstanding work is committed. Still no
      remote; add one with `git remote add origin <path-or-url>` when there
      is somewhere to push.

- [x] add new columns in models screen :
    - [x] for optimizations like QAT or other → **OPT** column: `qat`, `ud`
          (Unsloth dynamic quantisation), `moe` (read from the `A3B`/`A4B`
          active-parameter count).
    - [x] for model capabilities → **CAPS** column: `V` vision, `S`
          speculative decoding, `A` audio, `C` code. Uppercase = in use,
          lowercase = the model has it but this preset turns it off. The
          selected row is spelled out in full in the footer, which doubles
          as the legend.
    - [x] suggestions, and what was deliberately left out:
        - **reflection/thinking was dropped.** `reasoning = off` is set on
          *every* preset in both tiers, so a column driven by it reads the
          same on every row. It is a setting, not a capability — the
          Settings screen is where it belongs.
        - **vision under-claims on purpose.** `no-mmproj = true` looks like
          evidence of a projector, but it is set defensively here — including
          on Phi-4-mini and Nemotron-3-Nano, which have no vision at all. It
          now only decides whether a capability found by *name* is switched
          on. Cost: gemma-4 ships an mmproj without saying so in its name, so
          it reads as text-only. Fixing that properly means looking for an
          `mmproj` in the repo listing rather than guessing harder.
        - **voice/audio detects nothing today.** No shipped preset has it;
          the detector is there so one would light up without a code change.
        - The table now **adapts to the terminal width** — it was already 89
          columns wide and silently clipping on a 100-column terminal.
          Columns are dropped in a stated order (ctx → opt → caps → ram)
          rather than falling off the right edge.
        - Still worth considering: a **QUANT** column (`UD-Q4_K_XL` vs
          `Q4_K_M` vs `Q1_0`) — currently only visible when the repo column
          is wide enough not to elide it.

- [x] add a graceful exit
    - [x] ask confirmation on exit query if some actions are in progress
          (download, chat probe, running command) with a force option:
          `q` asks and names what would be abandoned, `Q` quits at once.
          A running server is *not* a reason to ask — it is stopped on exit
          every time by design.
    - [x] shutdown also kills an in-flight `hf` download (a partial blob
          resumes next time), draws a "shutting down…" frame before waiting,
          and every step is bounded.

- [ ] update rust toolchain to lastest stable one
- [ ] add a robust software versioning feature (increment at release build)

- [ ] how to optimize running memory and CPU consumption
