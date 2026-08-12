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

Until this lands, extension happens by adding an entry to the table in
`commands.rs` and a dispatch arm on the path that entry names. See
`architecture.md` for the three existing dispatch paths — and note that the
table is not documentation alongside the code but is checked against it: a
listed command that nothing handles fails the conformance tests.
