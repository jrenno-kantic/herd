# Plugin System

**Status: not implemented.** No plugin trait, registry, or loader exists in the
code. This file records the intent only.

## Goal

Allow dynamic extension.

## Interface (proposed)

Plugin must implement:
- `name()`
- `execute(args)`

## Examples

- network scanner
- automation workflows
- device integration

## Note

Until this lands, extension happens by adding a `CommandSpec` to
`services/scripts.rs` and a dispatch arm in `engine/executor.rs`. See
`architecture.md` for the three existing dispatch paths.
