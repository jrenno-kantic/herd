//! Who this build is.
//!
//! The three facts come from two places: `CARGO_PKG_VERSION` from
//! `Cargo.toml`, and the commit stamp from `build.rs`. Keeping them
//! together here means the CLI, the sidebar and a bug report all quote the
//! same string.

/// `0.1.0`
pub const NUMBER: &str = env!("CARGO_PKG_VERSION");
/// `a1b2c3d`, or `a1b2c3d-dirty`, or `unknown`.
pub const COMMIT: &str = env!("HERD_COMMIT");
/// `2026-08-11`, or `unknown`.
pub const COMMIT_DATE: &str = env!("HERD_COMMIT_DATE");

/// `herd 0.1.0 (a1b2c3d 2026-08-11)` — what `--version` prints and what a
/// bug report should carry.
pub fn long() -> String {
    format!("herd {NUMBER} ({COMMIT} {COMMIT_DATE})")
}

/// `0.1.0` on its own, or `0.1.0*` when the build came from a dirty tree.
///
/// The asterisk is the one bit of the stamp worth spending characters on
/// in the sidebar: a build with uncommitted changes is not a build anyone
/// else can reproduce, and that is easy to forget while working.
pub fn short() -> String {
    if COMMIT.ends_with("-dirty") {
        format!("{NUMBER}*")
    } else {
        NUMBER.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_long_form_carries_everything_a_bug_report_needs() {
        let long = long();

        assert!(long.starts_with("herd "), "{long}");
        assert!(long.contains(NUMBER), "{long}");
        assert!(long.contains(COMMIT), "{long}");
        assert!(long.contains(COMMIT_DATE), "{long}");
    }

    /// The stamp must be present even where git is not — a source tarball
    /// still has to build and still has to say something.
    #[test]
    fn the_stamp_is_always_populated() {
        assert!(!COMMIT.is_empty());
        assert!(!COMMIT_DATE.is_empty());
    }

    /// A dirty build is worth flagging where it will be seen, since it is
    /// one nobody else can reproduce.
    #[test]
    fn a_dirty_build_is_marked_in_the_short_form() {
        assert_eq!(short().ends_with('*'), COMMIT.ends_with("-dirty"));
        assert!(short().starts_with(NUMBER));
    }
}
