//! What herd remembers between runs, in `~/.herd_config`.
//!
//! A dotfile in `$HOME`, deliberately: it is meant to be opened, read and
//! edited by hand, which is also why the JSON is pretty-printed and every
//! map is a `BTreeMap` — a file that reshuffles its own keys on each write
//! is unreadable in a diff.
//!
//! Three things live here, and the boundary is worth stating:
//!
//! - **favourites**, which are purely a display preference;
//! - **setting overrides**, which used to die with the process. The rule
//!   they were written under is unchanged — `models.ini` is hand-written
//!   and commented, and herd must never rewrite it — but that rule was
//!   always about the *ini*, not about forgetting. They are kept in a file
//!   herd owns instead, so a preset tuned once stays tuned, and the ini is
//!   still the untouched thing the Settings screen shows them against.
//! - **router settings** (`--models-max`, `--sleep-idle-seconds`), which
//!   the Router screen edits.
//!
//! Both favourites and overrides are keyed by **preset name, not by
//! tier**: `gemma4-12b` appears in both shipped tiers and is the same
//! model, so a tuning done in one is the right answer in the other.
//!
//! Reading never fails: a missing, unreadable or corrupt file means "no
//! preferences yet", because losing a convenience must not stop the
//! program from starting. **Writing does report failure**, unlike
//! `session.rs` — a silently dropped save loses work the user did on
//! purpose, which is a different thing from forgetting which tier they
//! were on.

use super::overrides::Overrides;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// How many models the router keeps resident, and how long one may sit
/// idle before it is unloaded. The same defaults the `:router` command
/// has always used, shared so the command and the screen cannot disagree.
pub const DEFAULT_MODELS_MAX: u32 = 2;
pub const DEFAULT_SLEEP_IDLE_SECONDS: u32 = 300;

/// Bounds for the Router screen's `+`/`-`. Wide enough not to be in
/// anyone's way, narrow enough that a held key cannot produce a number
/// that would be rejected by llama-server or silently swallow the machine.
pub const MODELS_MAX_RANGE: (u32, u32) = (1, 16);
pub const SLEEP_IDLE_RANGE: (u32, u32) = (0, 3600);
/// One `-`/`+` press. Idle time steps in half-minutes because the useful
/// range spans an hour and stepping it a second at a time is not editing,
/// it is waiting.
pub const SLEEP_IDLE_STEP: u32 = 30;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Prefs {
    /// Preset names marked with a star, sorted and de-duplicated by the
    /// set itself so the file cannot grow duplicates.
    #[serde(default)]
    pub favorites: BTreeSet<String>,
    /// Presets with the `[mono-focus]` profile switched on. Keyed by name
    /// like the favourites, and `#[serde(default)]` like everything else
    /// here, so a file written before this existed still reads.
    #[serde(default)]
    pub mono_focus: BTreeSet<String>,
    /// Setting overrides, in the same shape `App` holds them.
    #[serde(default)]
    pub overrides: Overrides,
    #[serde(default)]
    pub router: RouterPrefs,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RouterPrefs {
    #[serde(default = "default_models_max")]
    pub models_max: u32,
    #[serde(default = "default_sleep_idle")]
    pub sleep_idle_seconds: u32,
}

fn default_models_max() -> u32 {
    DEFAULT_MODELS_MAX
}

fn default_sleep_idle() -> u32 {
    DEFAULT_SLEEP_IDLE_SECONDS
}

impl Default for RouterPrefs {
    fn default() -> Self {
        Self {
            models_max: DEFAULT_MODELS_MAX,
            sleep_idle_seconds: DEFAULT_SLEEP_IDLE_SECONDS,
        }
    }
}

impl Prefs {
    /// Reads `~/.herd_config`, or the defaults when there is nothing
    /// readable there.
    pub fn load() -> Self {
        path()
            .map(|path| Self::load_from(&path))
            .unwrap_or_default()
    }

    /// The same, from an explicit path. Every test uses this rather than
    /// `load`: a test suite that reads and writes the developer's real
    /// preferences is a test suite that can destroy their favourites.
    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = path().ok_or_else(|| "no HOME to write ~/.herd_config into".to_string())?;
        self.save_to(&path)
    }

    /// Written to a temporary file and renamed into place, so an
    /// interrupted write cannot leave a truncated preferences file where a
    /// complete one used to be. A rename within a directory is atomic.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        let temporary = path.with_extension("writing");

        std::fs::write(&temporary, format!("{text}\n"))
            .map_err(|error| format!("{}: {error}", temporary.display()))?;
        std::fs::rename(&temporary, path).map_err(|error| format!("{}: {error}", path.display()))
    }
}

/// `~/.herd_config`.
fn path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".herd_config"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::llama::overrides::Scope;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "herd-prefs-{}-{}-{name}.json",
            std::process::id(),
            name.len()
        ))
    }

    fn sample() -> Prefs {
        let mut overrides = Overrides::default();
        overrides.set(Scope::Global, "", "port", "8080");
        overrides.set(Scope::Model, "gemma4-12b", "ctx-size", "65536");

        Prefs {
            favorites: ["gemma4-12b".to_string(), "qwen3-coder".to_string()].into(),
            mono_focus: ["qwen3-coder".to_string()].into(),
            overrides,
            router: RouterPrefs {
                models_max: 3,
                sleep_idle_seconds: 600,
            },
        }
    }

    #[test]
    fn everything_survives_a_round_trip_through_the_file() {
        let path = temp_path("round-trip");
        let prefs = sample();

        prefs.save_to(&path).expect("save");
        let back = Prefs::load_from(&path);

        assert_eq!(back, prefs);
        assert!(back.favorites.contains("gemma4-12b"));
        assert_eq!(
            back.overrides.get(Scope::Model, "gemma4-12b", "ctx-size"),
            Some("65536"),
            "a tuned preset must still be tuned next session"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// The one thing that must never happen at start-up: refusing to run
    /// because the preferences file is missing or has been mangled.
    #[test]
    fn a_missing_or_corrupt_file_reads_as_no_preferences() {
        let missing = temp_path("nonexistent");
        let _ = std::fs::remove_file(&missing);
        assert_eq!(Prefs::load_from(&missing), Prefs::default());

        let corrupt = temp_path("corrupt");
        std::fs::write(&corrupt, "{not json at all").expect("write");
        assert_eq!(Prefs::load_from(&corrupt), Prefs::default());

        let _ = std::fs::remove_file(&corrupt);
    }

    /// A file written by an older herd — or edited by hand down to the one
    /// key someone cared about — must still load, and must not read the
    /// router's absence as "zero models, unload immediately".
    #[test]
    fn missing_sections_fall_back_to_the_defaults() {
        let prefs: Prefs = serde_json::from_str(r#"{"favorites": ["gemma4-12b"]}"#).expect("parse");

        assert!(prefs.favorites.contains("gemma4-12b"));
        assert_eq!(prefs.router.models_max, DEFAULT_MODELS_MAX);
        assert_eq!(prefs.router.sleep_idle_seconds, DEFAULT_SLEEP_IDLE_SECONDS);
        assert!(prefs.overrides.is_empty());
    }

    /// The write is not allowed to destroy the previous file when it
    /// fails, which is what writing in place would risk.
    #[test]
    fn a_save_leaves_no_temporary_behind() {
        let path = temp_path("temporary");
        sample().save_to(&path).expect("save");

        assert!(path.is_file());
        assert!(
            !path.with_extension("writing").exists(),
            "the temporary file outlived the save"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_preferences_file_is_a_dotfile_in_the_home_directory() {
        let path = path().expect("no HOME in this env");

        assert!(path.ends_with(".herd_config"), "{}", path.display());
        assert_eq!(
            path.parent().map(Path::to_path_buf),
            std::env::var("HOME").ok().map(PathBuf::from),
            "it must sit in $HOME, not in a nested application directory"
        );
    }
}
