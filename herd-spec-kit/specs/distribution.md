# Distribution Specification

## User-facing contract

- The supported Homebrew install command is
  `brew install jrenno-kantic/tap/herd-llm`.
- The formula name is `herd-llm`; it installs the executable as `herd`.
- Homebrew installs `llama.cpp` and `hf` as runtime dependencies. HERD still
  performs its startup preflight because an installed dependency can later be
  removed, broken, or hidden from `PATH`.
- Stable releases are announced by annotated `vX.Y.Z` tags, and the tag must
  match the Cargo package version.
- Prereleases must not replace the stable Homebrew formula. Homebrew exposes
  one current version of a formula, so publishing a prerelease would displace
  the latest stable release.

## Current implementation

The public tap is
[`jrenno-kantic/homebrew-tap`](https://github.com/jrenno-kantic/homebrew-tap).
Its formula downloads the tagged HERD source and builds it with Cargo. This is
the working fallback and remains authoritative until the prebuilt pipeline has
completed the acceptance checks below.

## Active release implementation: dist

Use [`dist`](https://axodotdev.github.io/cargo-dist/) (formerly cargo-dist) as
the release and Homebrew packaging helper. It is a good fit because it builds
per-target archives, hosts them on the GitHub release, generates a formula that
downloads those archives, and can publish that formula to the existing tap.
It does not generate source-building Homebrew formulae, so this is a deliberate
migration to prebuilt artifacts rather than a transparent refactor of the
current formula.

## Rollout status

- `dist` 0.32.0 configuration and generated GitHub release CI are checked in.
- `HOMEBREW_TAP_TOKEN` is configured on the HERD repository.
- `v0.8.8-rc.2` successfully built and published checksummed archives for
  Apple Silicon macOS, Intel macOS, and x86_64 Linux.
- The downloaded Apple Silicon artifact reports
  `herd 0.8.8-rc.2 (bbd51e3 2026-08-18)`.
- The generated formula is named `herd-llm` and declares `hf` and `llama.cpp`.
- Prerelease Homebrew publication was skipped, proving that the stable `0.8.7`
  source formula remains unchanged.
- Local `make verify`, Ubuntu Verify and macOS Verify all pass as of
  2026-08-21. The two macOS timeouts in `a_healthy_process_transitions_to_serving`
  and `stopping_then_launching_another_model_switches_cleanly` were a test
  backstop, not a product fault: both spawn a real `python3` HTTP server, and
  15s was comfortable on a developer machine but not on a contended hosted
  runner. The backstop is now `READY_BACKSTOP` (60s), which costs a healthy run
  nothing — the same macOS job that failed in 1m55s passes in 3m28s. Both tests
  also report the state and phase they stalled in, so a future failure says
  whether the server never bound or simply needed longer.
- A stray lightweight `v0.8.9` tag was pushed at a commit whose `Cargo.toml`
  still read `0.8.8-rc.4`. dist refused it at the `plan` step
  (`--tag=v0.8.8-rc.4 will Announce: herd`) and stopped before building, so
  `publish-homebrew-formula` never ran and the tap was untouched. That is the
  version-match rule in the user-facing contract doing its job, and it is worth
  keeping in mind: the guard is dist's, not the tag's, and it only fires after
  the tag is public. Delete and recreate rather than force-moving a bad tag.
- The next stable tag performs the first automated tap update and tap-native
  Homebrew validation. The source formula remains the rollback path until that
  publication has been validated by the tap's own workflow.

The intended generated configuration is equivalent to:

```toml
[dist]
cargo-dist-version = "0.32.0"
rust-toolchain-version = "stable"
ci = "github"
installers = ["homebrew"]
tap = "jrenno-kantic/homebrew-tap"
formula = "herd-llm"
publish-jobs = ["homebrew"]
publish-prereleases = false
targets = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "x86_64-unknown-linux-gnu",
]

[dist.dependencies.homebrew]
hf = { stage = ["run"] }
"llama.cpp" = { stage = ["run"] }
```

`dist init` owns the exact checked-in configuration and
`.github/workflows/release.yml`; the generated files must not be hand-written
from this example. Pin the tool version that generated the workflow, and use
`dist init` again to upgrade it so configuration and CI move together.
The explicit release toolchain is required because a hosted runner may expose
an older preinstalled Cargo that does not honor the repository's rustup
override before dist invokes it.

## Credentials and permissions

Publishing crosses repository boundaries: the workflow runs in
`jrenno-kantic/herd` and writes the generated formula to
`jrenno-kantic/homebrew-tap`. Configure a fine-grained token as the HERD
repository secret `HOMEBREW_TAP_TOKEN`, limited to contents write access on the
tap repository. The normal `GITHUB_TOKEN` remains limited to the HERD release.
Never place the token in Cargo metadata, workflow YAML, logs, or the tap.

## Release flow

1. Run `make verify` and cut the versioned release commit and annotated tag
   with `make release`, `make release-minor`, or `make release-major`.
2. Push the release commit first and require the independent Verify workflow to
   pass on Linux and macOS. The generated Release workflow does not depend on
   Verify and cannot enforce this gate itself.
3. Push the `vX.Y.Z` tag only after that branch verification is green.
4. The generated dist workflow builds archives for each declared target,
   creates or updates the GitHub release, and publishes only a stable release
   to the tap.
5. The tap's own workflow validates the formula independently.
6. If publishing fails after the GitHub release succeeds, keep the previous
   formula available and rerun or repair the publishing job; do not replace it
   with an unverified manual checksum update.

## Acceptance checks

Before switching the tap from its source formula to the generated formula:

- `dist plan` accepts the release configuration and reports the expected
  targets, Homebrew installer, and publish job.
- A release candidate created from a tag contains all declared archives and
  checksums, and `herd --version` matches the tag on each supported platform.
- The generated formula retains the MIT license and runtime dependencies on
  `llama.cpp` and `hf`.
- `brew style`, `brew audit`, `brew install`, `brew test`, and
  `brew linkage --test` pass for `jrenno-kantic/tap/herd-llm`.
- A clean machine can launch HERD far enough for startup preflight to find
  `llama-server`, and `:about` reports both external tools.
- The source-building formula remains recoverable from tap history as the
  rollback path.

## Scope boundary

dist owns binary archives, GitHub release assembly, and formula publication.
It does not replace HERD's version policy, local verification, startup tool
checks, or the tap's Homebrew-native CI. Code signing and Apple notarization
are separate distribution work; prebuilt archives must not be described as
signed or notarized until those steps exist and are verified.
