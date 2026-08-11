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

- [x] update rust toolchain to lastest stable one → 1.95.0 → **1.97.1**,
      green on build/test/clippy. Pinned with `rust-toolchain.toml`
      (`channel = "stable"`) and `rust-version = "1.97"` in Cargo.toml.
      Edition stays 2021 on purpose: edition 2024 makes `env::set_var`
      unsafe, which the config-resolution tests use — a separate,
      deliberate migration rather than a side effect of this.
- [x] add a robust software versioning feature (increment at release build)
    - `build.rs` stamps every binary with its commit and commit date, so
      builds between releases are still distinguishable. A dirty tree is
      marked `-dirty`.
    - `herd --version` → `herd 0.1.0 (a1b2c3d 2026-08-11)`; the sidebar
      shows `HERD 0.1.0`, with `*` when the build came from a dirty tree.
    - `make release` / `release-minor` / `release-major` verify, bump, tag
      and build. `VERSION=x.y.z make release` sets it outright.
    - **Not** auto-incremented on every release build, deliberately: a
      build script rewriting Cargo.toml dirties the tree on each
      `cargo build --release`, invalidates its own fingerprint and rebuilds
      in a loop, and numbers builds rather than releases. The commit stamp
      is what identifies an individual build.

- [x] fix model column spec as MTP has disappeared → **SPEC** is back as its
      own column, and now outlives CAPS when the terminal narrows: `S` says
      only *whether* speculative decoding is on, SPEC says which head.
- [x] split model description and keys usage on 2 lines (instead of one) →
      the Models footer is two lines now: what the highlighted preset *is*,
      then what the keys do. Sharing one line meant the description pushed
      `t/T tier` and `d download` off the right edge, and the description is
      the half that grows. The list loses a row for it, so `chrome(Models)`
      went 16 → 17.
- [x] disable "enter launch" when model is already started/serving in server
      screen → `LauncherState::relaunch_blocked` refuses, and the Server
      screen shows an `enter` field saying so, since a guard you only
      discover by pressing the key is the wrong way round. A *different*
      preset is still a legitimate hot-swap and goes through.
      **The Models screen deliberately still relaunches**: pressing Enter
      there after changing a setting is how a session override is applied.

- [x] add MTP in detialled model description line (Models screen) → the
      footer now reads `speculative decoding (mtp)`. The mechanism is
      carried on the trait itself (`caps::Trait::detail`) rather than
      special-cased in the renderer, and the `draft-` prefix is stripped
      since it is llama.cpp grouping, not information.
- [x] fix bonsai-27b (16gb tier) that announce 16.1G instead of 3.5G →
      **16.1G → 4.8G.** `Q1_0` matched no branch in `bits_per_weight` and
      fell through to a Q4-shaped default of 4.8 bits/weight, sizing a
      1-bit model as a 4-bit one. Q1 is now in the table at 1.2 bits
      (measured: `Bonsai-27B-Q1_0.gguf` is 3.54 GiB for 27B weights).
    - The real fix is the one behind it: **an unrecognised quantisation now
      reports nothing** instead of assuming Q4. Guessing a bit-width is
      guessing the answer, and the module already promised not to.
    - The width is read from the tag after `:`, not the whole reference, so
      a repo whose *name* mentions a width cannot override its build's.
    - Note 4.8G, not 3.5G: 3.54 is the file on disk, and the estimate adds
      the documented ~1 GiB runtime allowance on top of the weights.
      Every preset is sized that way.

- [x] why gemma4-31b is indicated as not present locally, but the download seems to fail silently
- [ ] fix when stop (s) is pressed while downloading a model, in that case reset the state as the app stay in error status
- [ ] how to optimize running memory and CPU consumption
