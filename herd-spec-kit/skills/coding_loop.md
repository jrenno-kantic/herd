# Coding loop

**Start green, leave green.** Before concluding any change:

```bash
cargo build && cargo test && cargo clippy --all-targets && cargo fmt
```

or `make verify`, which adds the format check and a release build.

Two conventions the tests enforce rather than merely state:

- **Nothing is documented in two places.** The keymap (`keys.rs`), the command
  table (`commands.rs`) and the shipped tiers (`data/`) are data, and each has a
  test that drives it against the code that runs it. A key that does something
  must be listed; a listed command must be dispatched; every shipped preset must
  parse and build a launchable argv.
- **Extract anything worth asserting on.** The pure functions — `lifecycle_glyph`,
  `wrap_argv`, `truncate`, `viewport_top`, `shell_command` — exist in that shape
  so they can be tested without a terminal, and `tui::render` is split from
  `TerminalSession::draw` so whole screens can be rendered against a headless
  backend.

The generation-era prompts that used to sit beside this file are gone: the
project they described building exists, and a prompt telling you to create a
sidebar is not a specification of anything.
