# OPS-TUI

Terminal-first control plane for power users.

## Features

- lazygit/k9s style UI
- command mode (:)
- logs panel
- modular architecture
- async command execution with non-blocking UI updates

## Run

```bash
cargo run
```

## Test

```bash
cargo test
```

## Keybindings

- `q` -> quit when the command input is empty
- `:` -> focus command mode
- `Tab` -> switch focus between sidebar and command input
- `Up` / `Down` -> navigate sidebar items
- `Enter` -> run the current command
- `Esc` -> clear command input and focus the sidebar

## Commands

Type commands in the command bar without the leading `:`. The UI displays the prompt for you.

- `help` -> show available commands and descriptions
- `test` -> run a sample predefined script
- `scan` -> scan for devices through the network service
- `sh <command>` -> run a shell command asynchronously (30s timeout; non-zero exits prefixed with `exit <code>:`)

## Use Cases

- Execute predefined operational scripts without leaving the terminal.
- Monitor command output in the logs panel while the UI remains responsive.
- Scan for local or remote devices from a keyboard-first control plane.
- Run quick shell checks through `sh <command>` and keep the result in the session log.
- Extend the service layer with future automation, network, or hardware integrations.

## Behavior notes

- Logs are capped at 500 entries; multi-line command output is split into separate entries before the cap is applied.
- Shell commands run via `sh <command>` are bounded by a 30s timeout. If the command fails or is killed, the log shows `exit <code>: <stderr>` or `timeout after 30s`.
- If a command task panics or is cancelled, the UI still receives a `task aborted` completion so the prompt never gets stuck in the running state.

## How To

1. Start the app with `cargo run`.
2. Press `:` to focus the command bar.
3. Type `help` and press `Enter` to list available commands.
4. Use `Up` and `Down` to move through sidebar sections.
5. Run `test`, `scan`, or `sh pwd` to see async results appear in the logs panel.
6. Press `q` with an empty command input to quit.
