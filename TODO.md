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
- [x] fix a bug when stop (s) is pressed while downloading a model, in that
      case reset the state as the app stay in error status → two causes:
    - **A race in the health poller.** It checked "is this launch still
      current", then spent up to the 3s health timeout waiting for a reply.
      A stop landing in that window announced OFF, and the poller then
      emitted on top of it — mid-download, an error about nothing
      listening. Reproduced with a listener that accepts and never answers:
      `[Starting, Stopping, Off, Starting]`. It now re-checks before
      emitting.
    - **No way out of ERROR.** `s` did nothing, because `is_live()` is
      false for a failed server, so the state outlived the failure it
      described. `s` now clears it — purely local, since nothing is
      running to stop.
- [x] how to optimize running memory and CPU consumption → **measured
      first, and mostly the answer is "it already is".**
    - Idle footprint: **10.5 MB RSS, stable**, against a llama-server
      holding 8–17 GiB. Not worth attacking.
    - Idle CPU was **80 ms per 30 s** — the 250 ms tick forcing ~120 full
      redraws of a screen that had not changed. A tick now only redraws
      when a clock is actually on screen (`App::ticking`): **80 ms → 30 ms**,
      reproducible over two runs.
    - Tried and **reverted**: trimming tokio to the features herd uses.
      Identical binary size, RSS and thread count — Cargo unifies features
      across reqwest anyway, so it bought nothing and the diff was noise.
    - Left alone: the 50 ms keyboard poll (raising it trades input latency
      for a few wakeups) and the 17 runtime threads (they cost virtual
      address space, not RSS). Neither is justified by the measurements.

- [x] Add an option to handle favorite models with a golden star
  - [x] Add toggle favorite feature on models → `f` stars the highlighted
        preset and takes the star off again; `★` is drawn in the marker
        field (now three wide: caret, star, lifecycle glyph) in gold, and
        the star is a *glyph* rather than a colour so it survives a
        screenshot, a colour-blind reader and a terminal with its own
        palette. Not drawn in gold on the selected row, where gold on the
        selection's green is unreadable.
    - **The list is deliberately not reordered around favourites.** A
      table people navigate by position must not rearrange itself because
      a star was added above; there is a test for it.
    - `Columns::row` now pads and clips the marker itself. It was assumed
      to be exactly `W_MARKER` wide, and a row that supplies no glyphs at
      all — unstarred, unselected, not serving — would otherwise slide
      out of line with the header.
- [x] Add in app preferences file like ~/.herd_config (classical macosx way not local app files)
  - [x] Save custom model settings in this config file → `services/llama/
        prefs.rs`. A pretty-printed JSON dotfile in `$HOME`, sorted keys,
        meant to be read and edited by hand.
    - It holds favourites, the setting overrides and the router numbers.
      **The old "session-only" rule is unchanged where it mattered**:
      `models.ini` is hand-written and commented and herd still never
      writes to it. That was always about the *ini*, not about forgetting,
      so overrides now live in a file herd owns and a preset tuned once
      stays tuned.
    - Keyed by preset name, not by tier: `gemma4-12b` is in both shipped
      tiers and is the same model.
    - Reading never fails (missing/corrupt = no preferences yet, since
      losing a convenience must not stop start-up); **writing does report
      failure**, unlike `session.json` — after the alternate screen is
      restored, so the message is actually visible. Written via a
      temporary file and a rename, so an interrupted save cannot truncate
      the previous one.
    - Saved from `main.rs` on exit, like the session file, because
      `App::update` stays pure and does no I/O.
    - **Left out on purpose:** the memory reservation (Stats `+`/`-`)
      stays session-only. It is a property of the machine you are on at
      the moment, not a preset setting, and the TODO asked for model
      settings. Say the word and it is two lines.
- [x] Add a feature to start llama-server as router mode (with current tier)
  - [x] Add a router screen to specify router settings like models-max,
        sleep-idle-seconds → **screen 3**, straight after Server: it is
        the same lifecycle seen from the other end, one process that loads
        and unloads presets itself. `j/k` moves between the two settings,
        `+`/`-` to adjust (matching the Stats screen's reservation rather
        than opening an edit mode for a single number), `enter` starts the
        router with exactly what is on screen, `s` stops it, `r` resets,
        `y` copies the command.
    - It carries the same live **argv preview** as the Models screen: the
      two numbers only mean anything as the flags they become, and
      `:router --max N --idle S` passed them where nobody could see them.
    - The state line reports the supervised process **only when it is the
      router**. A single preset serving from the Models screen is not this
      screen's business, and showing SERVING for it would claim the router
      was up when it is not.
    - Defaults (2 models, 300s) are now shared with `parse_router_flags`,
      so a typed `:router` and an untouched screen cannot disagree.
    - Inserting a screen renumbered `1`–`7`. The tests that navigated by
      digit now derive the digit from `Screen::ALL`: hard-coding `3` does
      not *fail* when a screen is inserted, it quietly starts testing a
      different screen.
    - **Not done:** the port-in-use prompt still does not cover router
      mode (it never did for `:router`); a busy port surfaces as
      llama-server's own bind error in the log.

- [x] Screen footers now fit the pane they are drawn in
      (`keys::screen_hint_within`). The hints outgrew the line the moment
      a screen gained a seventh key — at 100 columns the Models footer had
      74 and wanted 76 — and a footer that is too long does not wrap, it
      silently loses its last hints off the right edge, which is exactly
      the dishonesty `Columns::for_width` exists to prevent for the table.
      Hints are dropped from the end, least load-bearing first, and the
      line ends in `…` to say so. The marker is an ellipsis rather than
      "? more" because those six characters cost the Models footer its
      tier key, and `?` is already named in the status bar.
  
- [x] Add a right slider in models screen to see the position of the content
      (visible only if the screen is too small to display all items) →
      `components::list_scrollbar`, drawn on the right border and only when
      there are more rows than fit, exactly as asked. The Hub screen uses
      it too.
    - It is drawn against the **list rows**, not the whole pane, so the
      thumb spans what it describes rather than the column header and the
      two footer lines as well.
    - The position is `viewport_top`, which *reproduces* what ratatui's
      `List` does rather than guessing at it: ratatui owns the offset and
      does not expose it, but the `ListState` is rebuilt from the cursor
      every frame, so it always starts at 0 and scrolls only as far as it
      must. A bar derived from anything else would disagree with the rows
      beside it — which is worse than no bar.

- [x] Add an option to see locally installed HF hub models → **a screen,
      not an option**: `Hub`, inserted as screen 2, right after Models. The
      two answer halves of the same question — Models is what this tier can
      launch, Hub is what this machine has — and the list needs a cursor, a
      position, a footer and its own keys, which is a screen.
  - [x] Use colors to identify unreferenced models within the current tier
        → cyan, and named in the summary line ("3 not named by this tier").
        Cyan rather than red because it is not an error: a model this tier
        cannot launch may belong to another tier. Red and amber already
        mean "too large" and "tight" on a list row.
  - [x] Add an option to copy to the clipboard the required models.ini
        content needed to add this model → `y` copies a stanza
        (`[name]`, `hf-repo`, `alias`). Just those: `[*]` already carries
        the context size and the rest, and a stanza restating them would
        fight the defaults the file is built around. To the clipboard,
        never appended — `models.ini` is hand-maintained and herd does not
        write to it.
    - The screen shows **two** sizes, because there are two questions.
      `SIZE` is the weights llama.cpp would load; `DISK` is everything the
      repo holds. They are far apart: a repo keeps every revision it has
      ever fetched, so `gemma-4-12B-it-qat-GGUF` here is 6.3G of model in
      13.1G of directory. That gap is the reclaimable part, and it is the
      reason to look at this screen at all.
    - `*` on `DISK` means two cached quantisations share the directory.
      Said rather than divided: the cache keeps no per-quantisation
      accounting, and splitting the total would be inventing a number.
    - `enter` jumps to the preset that names the model, since everything
      that acts on a preset already lives on the Models screen. `r` (and
      `:cache`) re-reads the cache, which changes underneath herd — a
      download in another terminal, a repo deleted to free space.
    - **Deliberately no delete key.** Freeing 17 GiB is not something to
      offer one keystroke away from `j`, and herd does not touch what it
      did not put there — the same restraint as printing the `sysctl` line
      on the Stats screen rather than running it.
    - Inserting a screen renumbered the digits to `1`–`8` again. The tests
      derive them from `Screen::ALL`, but a *string* in a component does
      not: "run a test on screen 3" had been pointing at the Router since
      that screen was inserted. It is derived now, and tested.

- [x] Add Time to First Token (TTFT) in stats → `first token  0.42s avg ·
      0.39s last · 0.31s best`, next to throughput, because the two come
      apart exactly where it matters: a model paging its weights in
      generates at a respectable rate and still shows nothing for four
      seconds.
    - **Derived, and it says so.** The probe is non-streaming on purpose
      (it is `test_call.sh`'s request, so the two stay comparable), so
      nothing sees the first token arrive. What is known is the round trip,
      measured locally, and llama.cpp's own `predicted_ms`; the difference
      is queueing, prompt ingestion and the network — the wait being asked
      about. A server that sends no `timings` gets `-` and a reason, not a
      zero.
    - `best` is the **shortest**, unlike the best rate. Counted over its
      own probe count, since a server that reports no timings still
      answers and averaging it in would halve the figure.

- [x] Ensure the displayed models size are correct in models screen →
      **a downloaded preset is now measured, not estimated**: the gguf of
      its quantisation in the revision `refs/main` names, resolved through
      the snapshot symlinks, plus the same runtime allowance the estimate
      adds so both kinds of row can be judged against the same budget.
    - The table marks which is which: `~18.3G` is arithmetic on the repo
      name, `7.3G` is a file that was read. The heuristic has been wrong by
      a factor of four before (`Q1_0` sized as a Q4) and the two are not
      worth showing as equally certain.
    - **Summing the blobs would have been the obvious and wrong fix**: a
      repo keeps every revision it has fetched, so that announces a 12B
      model at twice its size. The snapshot names exactly one file.
    - The quantisation has to match, unlike `availability`, which accepts a
      repo cached under any tag: sizing a cached Q4 against a preset asking
      for Q8 would be a confident, specific, wrong number.
    - **Not done:** a preset that is *not* downloaded is still estimated.
      The honest fix there is the HuggingFace tree API, which is a network
      call per row — worth doing only if the estimate proves wrong on a
      preset someone actually cares about.

- [x] Add a help command in the command component to display information
      about all available commands → `:help` opens a **Commands** overlay,
      grouped (llama-server / models.ini / other) and showing each command
      with its arguments — `ping` without a model is a usage error, not a
      command. The counterpart to `?`, which does the same for keys, and
      each points at the other.
    - **An overlay, not a log line.** `:help` was already dispatched like
      any other command and printed its list into the log — which is on
      another screen, is where a loading server writes hundreds of lines,
      and scrolls. Being answered somewhere else, later, is not an answer.
    - Answered locally in `App::submit_command` and **before the busy
      gate**, like `stop`: "what can I type" is a question people ask
      *because* something is stuck.
    - The list that existed (`scripts.rs::COMMANDS`) had drifted from the
      dispatchers, which is what made this worth doing properly rather
      than reformatting: it never learned `reload`, never learned `cache`,
      and told the reader the Test screen was "key 3" — untrue since the
      Router screen was inserted. `commands.rs` is now the only place a
      command is written down.
    - It is **checked against the dispatchers**, the same bargain
      `keys.rs` makes: each entry carries a handler and a probe, and two
      tests split the table between them, failing if a documented command
      ever comes back "Unknown command".
    - The bar carries the pointer on its border, and while a line is being
      typed it names what that line would run — `:lauch` reads as
      `unknown` before Enter rather than after it.
    - `launch!` is in the table but not listed: the port-in-use prompt
      emits it, and advertising it would invite skipping a check that
      exists for a reason.
    - The overlay sizes itself to its content and elides visibly when the
      terminal is narrower, since a clipped summary reads as one that
      simply ended there.

- [x] increase version number (at least patch level) on every commit →
      `hooks/pre-commit`, installed with `make hooks` (it points
      `core.hooksPath` at the versioned `hooks/` directory, so the script
      lives with the code rather than unbacked-up in `.git/hooks`).
    - **A hook, not `build.rs`.** A build script rewriting `Cargo.toml`
      dirties the tree on every build, invalidates its own fingerprint and
      rebuilds in a loop, and counts builds rather than changes — the
      reason this was refused when it was asked for as a build-time
      feature. A commit is a discrete act with somewhere to hook into, so
      per-commit is the version of the request that works.
    - **A commit that already sets a version is left alone**, which is
      what keeps `make release` meaningful: a deliberate 0.8.0 lands as
      0.8.0, not 0.8.1. `HERD_NO_BUMP=1` skips one, `make hooks-off`
      stops it.
    - It **refuses rather than staging `Cargo.toml` wholesale** when that
      file has unstaged edits: sweeping uncommitted work into a commit is
      the one way a hook like this does real damage.
    - The lock is rewritten with awk rather than by running cargo — a hook
      that compiles is a hook people turn off.
    - Verified in a throwaway repo before being installed here: bumps an
      ordinary commit (both files), leaves a deliberate version alone,
      honours the escape hatch, and refuses the unstaged case.

- [x] set version to 0.7.0 → from 0.5.0, carrying the Hub screen, measured
      sizes, TTFT, the list scrollbar and `:help`.

- [x] are `:sh`, `:test` and `:scan` still required? → **`test` and `scan`
      are gone; `sh` stays.** Both removed answered fixed strings:
      `test` returned "Test executed", and `scan` returned "No devices
      discovered" after a 500 ms sleep, having never looked at a network.
    - They were harmless as pre-pivot vestiges right up until `:help`
      started advertising them. A listing that promises a network scanner
      there is no scanner for is the same dishonesty `Columns::for_width`
      and `screen_hint_within` exist to prevent — and `:test` collided
      with the Test screen, which is a different thing entirely.
    - `sh <command>` is the one that does what it claims: a real
      `sh -c` bounded at 30s, and `:sh ls ~/models` without leaving the
      alternate screen is a genuine use.
    - `services/network.rs` deleted with `scan` — it existed only for it.
      The plugins spec still lists a network scanner as *intent*, which is
      honest in a file that says "not implemented" at the top.
    - A test now pins that both retired names answer "Unknown command":
      a command that is no longer listed and still runs is the same drift
      `commands.rs` prevents, pointing the other way.

- [x] move Hub to the last entry in the menu → `Screen::ALL` reordered,
      which is the only place the menu order lives, so it was a one-line
      change. The digits follow (`8` is Hub now), and nothing needed
      renumbering by hand because no test and no component hard-codes one.
    - The order now follows **how often a screen is wanted** rather than
      how closely it relates to its neighbour: the first seven are what a
      session moves through, Hub is where you go occasionally.

- [x] increase the Hub list's first column width → two changes, because
      raising the cap alone would have done nothing at 100 columns.
    - `REF_MAX` 56 → 76, wide enough for the longest reference on this
      machine (the 73-character huihui one), so a wide terminal shows it
      whole.
    - **The fit order was backwards.** It shrank the reference to its
      minimum *before* dropping anything, so a 100-column terminal showed
      `…h/gemma-4-12B-it-qat-GGUF:Q4_K_XL` while still finding room for a
      disk figure. The reference now gives ground only to `REF_COMFORT`
      (42 — a full `unsloth/…:Q4_K_XL` is 39), then DISK goes, then it
      gives the rest, and the preset name goes last. At 100 columns nine
      of eleven references now read in full.
    - A test walks every width and fails if a column was dropped while
      more room than it needed went unused — the mistake a fixed drop
      order invites at the width where both would have fitted.

- [x] TTFT measured only on the first call after a model loads →
      `SessionStats::first_token`, recorded when `probes == 0` and never
      again; the screen reads `4.20s  (first call after loading)`.
    - That request is the only one that measures anything: it finds the
      weights cold and the cache empty. Every probe after it is answered
      by a resident, warm model, so the running average this replaces
      drifted towards the warm figure the more probes were run — and
      described neither.
    - A relaunch measures again, since `SessionStats` is reset on every
      `Starting` and the model is cold once more.
    - If that first probe's server sends no `timings` there is no cold
      measurement to be had this session: the screen says so rather than
      quietly substituting the second probe, which would be a warm number
      wearing a cold label.

- [x] are the initial (pre-pivot) features obsolete? clean the project of
      them → **yes, all of them; only `sh` survives**, and it was kept on
      purpose two steps ago because it does what it claims.
    - **Code.** The `help` arm in `run_script` could no longer fire —
      `:help` is intercepted in `App::submit_command` — and an unreachable
      fallback is not insurance, it is a claim nobody can check;
      `commands::help_text` went with it. So did a **300 ms `sleep`** at
      the top of `run_script`: it made a generated demo feel busy, and did
      nothing here but tax every `:sh` by a third of a second.
    - **The rename shims went too**, and this was checked rather than
      assumed: `$OPS_TUI_LLAMA_CONFIG` is unset and appears in no shell
      profile, and `~/.config/herd/session.json` is *newer* than the
      `ops-tui` one, so the fallback branch was already unreachable. The
      migration had been paid for. The stale file in `~/.config/ops-tui/`
      is inert and left alone — herd does not delete what it did not write.
    - **Docs and specs.** Deleted: `doc/prompt.md` (the multi-device
      méta-prompt, already headed "document historique"), the spec kit's
      `prompts/` (bootstrap / debug / features — the project they describe
      building exists), `skills/prompting.md`, and `specs/plugins.md` — a
      proposal with no trait, registry or loader, whose examples were the
      dead scope. `roadmap.md` now has a **Retired** section instead of a
      "Not started" one pointing at files that no longer exist.
    - `skills/coding_loop.md` was rewritten rather than deleted: the
      "start green, leave green" rule is live, and the numbered
      generate-run-debug list around it was not.
    - **Kept:** `data/` (llama reference scripts and test fixtures, not
      pre-pivot), and the TODO entries above — this file is a record of
      decisions, so its history stays even where the code has gone.

- [x] ensure doc and specs are up to date → `herd-spec-kit/` was two
      releases stale: it still listed six screens and no Router. Product,
      UX, architecture, data-model, services, roadmap, layout and theme are
      all current, and `spec-kit.yaml` now records which herd version it
      tracks. README, CLAUDE.md and the French handoff doc updated
      alongside, and kept current with every change since.

- [x] Add a delete model feature to the hub screen, including a
      confirmation prompt → `D` on the Hub screen. **The one destructive
      key in the program**, and it was deliberately absent until now: what
      makes it reasonable is the prompt asked for here, not a change of
      mind about the risk. Four fences:
    - **Uppercase `D`.** Lowercase `d` on the Models screen *downloads* —
      the same finger meaning "fetch this" on one screen and "destroy
      this" on the next is how an accident happens. Capitals are already
      the force variants (`Q`, `X`).
    - **The prompt states the cost before the question**: the size, and
      any other cached quantisation in the same directory that goes with
      it. A prompt that took a second model silently would not have asked
      the question the user answered.
    - **Only a lowercase `y` confirms**, unlike the launch prompts which
      also take `Y`. A slipped shift key must not be what destroys a
      download.
    - **Two outright refusals rather than warnings** — the repo a live
      server is serving from, and one a download is still writing into.
      Neither has a sensible "yes anyway".
    - The path is computed by `repo_dir`, never taken from typed text, and
      checked three ways before anything is unlinked (under the hub, the
      `models--` prefix, an existing directory). A test drives `""`, `..`
      and traversals at it and asserts a directory beside the hub survives.
    - **The whole repo goes, not one quantisation.** The cache keeps no
      per-quantisation accounting, and picking blobs out of a shared
      directory by hand is how a cache gets corrupted — so the prompt
      names what else goes instead of pretending to be surgical.
    - Afterwards the cache is **re-read** rather than assumed: a removal
      that half-succeeded shows up as a row still listed, not as a screen
      quietly disagreeing with the disk.
- [x] identify features requiring llama-server to be serving, and report
      that no server is running → **three of them**, and all three now say
      so instead of failing in the plumbing's terms:
    - `:status` and `:ping <model>` (typed, and `p` on the Server screen)
      — both reached the network directly and surfaced reqwest's own
      `error sending request for url (…)`, wrapped again by the Executor
      into `… unreachable: request failed: …`: two problems on one line,
      neither of which says the fix is to launch something.
    - The Test screen's probe already refused out loud (`is_live()`), and
      still does — but during STARTING the port may not be up yet, so it
      benefits from the same classification.
    - `api::unreachable` now does the classifying: a refused connection
      becomes "nothing is listening on <base> — no llama-server is running
      (start one with :launch <model>, or :router)"; a **timeout stays a
      timeout**, since something *did* answer the door and that is a
      different problem; anything else is flattened through its source
      chain rather than truncated at reqwest's surface message.
    - Refusal is detected **two ways** — `is_connect()` and the
      `io::ErrorKind` beneath — because the first has moved between
      reqwest releases and the second has not, and falling back silently
      to the plumbing message is the regression this replaced.
    - **Not refused up front**, even when herd knows its own state is
      `Off`: probing a server started outside herd is a supported use, and
      so is one that died a second ago. Attempt, then explain.
    - `p` on the Server screen was the silent case — no model name, so it
      returned `None` without a word, indistinguishable from an unbound
      key. It says so now.
    - Pinned end to end by `status_and_ping_say_that_no_server_is_running`,
      which drives both commands at a real closed loopback port (no
      network) and fails on either old phrasing.

- [x] Add an about command to show an About dialog box → `:about`, the
      third local overlay beside `?` (keys) and `:help` (commands),
      answering the third question a stuck user has: what am I running?
    - **Not decorative.** It is `--version` on screen — version, commit,
      commit date — plus the facts that decide behaviour on *this*
      machine: the loaded `models.ini`, its tier, the RAM detected and the
      budget that follows, and the cache directory. That is what a bug
      report is useless without.
    - Every line of it is already somewhere (sidebar, Models title, Stats
      screen), and that is the point: answering "what am I running?"
      should not be a tour of four screens.
    - The "uncommitted changes — not reproducible" line appears **only
      when it is true**, in the amber that means "worth noticing".
    - Values are elided from the **left**, since a path is identified by
      its end: `…/data/32gb/models.ini` says which config is loaded where
      `/Users/jrenno/Documents/dev…` says only whose machine it is. A test
      walks the lines and fails if one runs past the box.
    - The description comes from `CARGO_PKG_DESCRIPTION` rather than being
      written out again, so the dialog and the manifest cannot drift.
    - Answered locally and **before the busy gate**, like `:help` — same
      reasoning, and now covered by the same test.
    - It also caught the conformance test being too specific: it asserted
      `mode == Commands` where it meant "did something visible". A second
      overlay is exactly the change that assertion should have survived,
      so it now reads `mode != Browse`.

- [x] In Stats screen: rename "first token" to "TTFT", replace "(first call
      after loading)" with "(Time to First Token)", append the last and avg
      values → the line now reads:

      `TTFT   4.20s  (Time to First Token) · last 0.35s · avg 1.63s`

    - `last` and `avg` had to come **back**: measuring only the first call
      (the previous step) meant nothing tracked the warm model, and these
      are the other half of the question — the leading figure says how
      long until it was usable at all, these say what a request costs once
      it is.
    - **The leading figure is still cold**, and stays cold: a later, faster
      probe never replaces it. They are kept apart rather than merged into
      one number, since a single mean over both drifts towards the warm
      value the more probes are run and describes neither. There is a test
      for that.
    - The warm counters have their own probe count rather than reusing
      `probes`: a server that sends no `timings` still answers, and
      averaging it in as a zero would halve the figure.
    - **One thing the rename costs**, worth knowing: `(Time to First
      Token)` expands the acronym where `(first call after loading)` said
      which probe the leading number came from. That fact now lives only
      in the docs. Say the word and it can be a word on the line —
      `4.20s cold · last … · avg …` would carry both.