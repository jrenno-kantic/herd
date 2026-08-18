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

## Target implementation: dist

Use [`dist`](https://axodotdev.github.io/cargo-dist/) (formerly cargo-dist) as
the release and Homebrew packaging helper. It is a good fit because it builds
per-target archives, hosts them on the GitHub release, generates a formula that
downloads those archives, and can publish that formula to the existing tap.
It does not generate source-building Homebrew formulae, so this is a deliberate
migration to prebuilt artifacts rather than a transparent refactor of the
current formula.

The intended generated configuration is equivalent to:

```toml
[dist]
cargo-dist-version = "0.32.0"
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
2. Push the release commit and its `vX.Y.Z` tag.
3. The generated dist workflow builds archives for each declared target,
   creates or updates the GitHub release, and publishes only a stable release
   to the tap.
4. The tap's own workflow validates the formula independently.
5. If publishing fails after the GitHub release succeeds, keep the previous
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
