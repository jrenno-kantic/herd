# Repository Guidelines

## Project Structure & Module Organization

HERD is a single Rust 2021 crate. `src/main.rs` is the entry point; `src/app.rs` owns application state. Rendering is split between `src/tui.rs`, `src/layout.rs`, and screen modules under `src/components/`. Process execution lives in `src/engine/`; llama configuration, discovery, downloads, and lifecycle code live in `src/services/llama/`.

Reference model tiers and helper scripts are under `data/`. Product and architecture notes live in `herd-spec-kit/`; longer user-facing behavior belongs in `README.md`. Keep implementation tests beside the code they exercise in `#[cfg(test)]` modules—there is no separate `tests/` tree today.

## Build, Test, and Development Commands

- `make run`: launch the debug TUI; press `q` to exit.
- `cargo run -- --config data/16gb/models.ini`: run against a specific preset file.
- `make test`: run unit and Tokio async tests.
- `make lint`: run Clippy and treat warnings as errors.
- `make fmt`: format all Rust sources with `rustfmt`.
- `make verify`: run check, lint, format validation, tests, and a release build. Use this before opening a PR.
- `make help`: list all supported development and release targets.

The stable toolchain is declared in `rust-toolchain.toml`.

## Coding Style & Naming Conventions

Follow `rustfmt` output (four-space indentation) and keep Clippy clean. Use `snake_case` for modules, functions, and variables; `PascalCase` for types and traits; and `SCREAMING_SNAKE_CASE` for constants. Keep UI components focused on rendering/input and llama-specific behavior in `services/llama`. Prefer descriptive `anyhow` context over runtime panics.

## Testing Guidelines

Add focused `#[test]` or `#[tokio::test]` cases next to changed behavior. Name tests after outcomes, such as `reload_preserves_selection`. Cover parsing boundaries, lifecycle transitions, and narrow-terminal rendering regressions. No coverage threshold is enforced; use `make coverage` locally.

## Commit & Pull Request Guidelines

History uses concise Conventional Commit subjects: `feat(hub): ...`, `fix(download): ...`, `docs: ...`, and `refactor: ...`. Keep each commit scoped and written in the imperative mood. Install the repository hook with `make hooks`; it increments the patch version on each commit. Use `HERD_NO_BUMP=1 git commit ...` only for intentional exceptions such as fixups.

PRs should explain the user-visible change, link relevant issues or spec notes, and report `make verify` results. Include terminal screenshots for layout or interaction changes, plus any manual test configuration used.

## Configuration & Safety

Do not commit machine-local `~/.herd_config`, model weights, cache contents, or secrets. Treat cache deletion and spawned shell/process changes as destructive paths: retain explicit confirmation and add regression tests.
