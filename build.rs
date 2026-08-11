//! Stamps the binary with the commit it was built from.
//!
//! A version number alone cannot answer "which build is this?" — the same
//! `0.1.0` is emitted by every build between two releases, including a
//! working tree with uncommitted changes. The commit and its date close
//! that gap, so a bug report naming a build can be traced to a checkout.
//!
//! Everything here degrades rather than fails: a source tarball with no
//! `.git`, or a machine with no `git`, still builds and simply reports
//! `unknown`. A build script that can break the build over metadata is
//! not worth having.

use std::process::Command;

fn main() {
    // Without these the stamp is baked once and then goes stale for every
    // later build, which is worse than having no stamp at all.
    //
    // They catch commits and branch switches, which is what matters. They
    // do not catch a tree that goes from dirty back to clean without a
    // commit — `git checkout .` leaves a `-dirty` stamp until the next
    // rebuild. Watching the whole worktree to fix that would re-run this
    // script on every keystroke; a stale marker that errs towards "not
    // reproducible" is the cheaper mistake.
    for path in [".git/HEAD", ".git/refs/heads"] {
        println!("cargo:rerun-if-changed={path}");
    }

    println!("cargo:rustc-env=HERD_COMMIT={}", commit());
    println!("cargo:rustc-env=HERD_COMMIT_DATE={}", commit_date());
}

/// Short commit hash, marked `-dirty` when the tree has uncommitted
/// changes. The marker matters more than the hash: it is the difference
/// between a build someone else can reproduce and one they cannot.
fn commit() -> String {
    let Some(hash) = git(&["rev-parse", "--short", "HEAD"]) else {
        return "unknown".to_string();
    };

    // `git status --porcelain`, not `git diff --quiet`.
    //
    // `diff --quiet` does not refresh the index, so it reports a
    // difference for a file whose mtime moved but whose contents did not —
    // which `touch`, rustfmt and every editor save do routinely. It marked
    // a clean checkout `-dirty`, and a marker that is always on says
    // nothing. `status` refreshes; `--untracked-files=no` keeps scratch
    // files out of it, since the question is whether the *source that
    // built this* is committed.
    let dirty = capture(&["status", "--porcelain", "--untracked-files=no"])
        .is_some_and(|output| !output.trim().is_empty());

    if dirty {
        format!("{hash}-dirty")
    } else {
        hash
    }
}

/// The commit's own date rather than the build's: it is stable across
/// rebuilds of the same source, which is what makes two builds of one
/// commit comparable.
fn commit_date() -> String {
    git(&["log", "-1", "--format=%cd", "--date=short"]).unwrap_or_else(|| "unknown".to_string())
}

/// Runs git and returns its output, or `None` if it is absent or failed.
///
/// Empty output is a real answer here — it is how `status --porcelain`
/// says "clean" — so it is kept, unlike in [`git`].
fn capture(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
}

/// Runs git for a value, treating empty output as failure.
fn git(args: &[&str]) -> Option<String> {
    let text = capture(args)?.trim().to_string();
    (!text.is_empty()).then_some(text)
}
