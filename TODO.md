## TODO

- [x] Request confirmation before leaving when a download is in progress.
      `q` now opens the confirm prompt whenever `App::in_flight()` names
      anything — a download, a chat probe, a running command — as well as
      when a supervised server is live. The two conditions stay separate:
      a server is asked about but never *listed*, because stopping it on
      exit is the documented behaviour every time. The prompt adds one
      line when a download is what is at stake, saying it resumes from
      where it stopped, which is true (`hf` writes into a `.incomplete`
      blob) and is the difference between a decision and a scare. `Q`
      still forces. Left out: no attempt to *continue* a download past
      exit — that would mean a detached process herd no longer supervises.
- [x] Increase the timeout for the sanity check regarding the existence of
      llama and hf tools.
      `PROBE_TIMEOUT` is 15s, up from 3s. Three seconds was enough on a
      warm machine and not on a cold one, and both failures are expensive:
      `llama-server` is required, so a false timeout aborts before the TUI
      opens, and a false timeout on `hf` disables downloads for the whole
      session. A tool that is simply absent still fails instantly — it
      fails on `spawn`, not on the timeout — and the two probes run
      concurrently, so this is the worst case for start-up as a whole
      rather than per tool. The message now names the constant instead of
      a hard-coded "3s".
- [x] On the models screen, include a feature to access the OpenCode
      configuration section to add support for this model.
      `o` opens a read-only overlay with the `opencode.json` provider
      block for the highlighted preset; `y` copies it and closes. Built
      from `LaunchSettings::argv`, so the endpoint, the alias and the
      context size are the ones a launch would really use, Settings-screen
      overrides included. Fields herd cannot support are omitted rather
      than guessed (`tool_call` only with `--jinja`, `attachment` only
      with vision switched on, `limit` only with a context size). Left
      out: herd does not write `~/.config/opencode/opencode.json` — it
      belongs to another program, and rewriting it would lose its comments
      and key order, the same rule that keeps `models.ini` untouched.
