## TODO

- [x] rename ops-tui to a more appropriate name → **herd** (a herd of llamas).
      `$OPS_TUI_LLAMA_CONFIG` and `~/.config/ops-tui/session.json` are still
      read as fallbacks, so the rename costs no remembered state.
- [x] add git local repo → the outstanding work is committed. Still no
      remote; add one with `git remote add origin <path-or-url>` when there
      is somewhere to push.

- [ ] add new columns in models screen :
    - [ ] for optimizations like QAT or other 
    - [ ] for model capabilities (reflection/thinking, vision, voice...)
    - [ ] Do you have any suggestions ?

- [ ] add a graceful exit
    - [ ] ask confirmation on exit query if some actions are in progress (like download, processing http queries) with a force option (if required)

- [ ] add a robust software versioning feature (increment at release build)
- [ ] how to optimize running memory and CPU consumption
