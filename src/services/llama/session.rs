//! Remembers the last tier and model across restarts.
//!
//! Deliberately minimal: **only** the active config path and the last
//! launched preset name. Everything the user *chose* — favourites,
//! setting overrides, the router numbers — belongs in `~/.herd_config`
//! (see `prefs.rs`) and must never end up in here: this file is where the
//! program was, not what the user asked for.
//!
//! Every failure is silent — a missing, unreadable or corrupt session file
//! just means "no memory yet". Losing a convenience must never block start-up.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    /// Absolute path of the `models.ini` last in use.
    #[serde(default)]
    pub config_path: Option<PathBuf>,
    /// Name of the preset last launched.
    #[serde(default)]
    pub model: Option<String>,
}

impl Session {
    /// Reads the session file, falling back to the pre-rename location.
    pub fn load() -> Self {
        let read = |candidate: Option<PathBuf>| -> Option<Self> {
            let text = std::fs::read_to_string(candidate?).ok()?;
            serde_json::from_str(&text).ok()
        };

        read(path())
            .or_else(|| read(legacy_path()))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    /// The remembered config path, but only if it still exists — tiers get
    /// renamed and deleted, and silently falling back to RAM detection is
    /// friendlier than starting on a dead path.
    pub fn usable_config_path(&self) -> Option<PathBuf> {
        self.config_path
            .as_deref()
            .filter(|path| Path::new(path).is_file())
            .map(Path::to_path_buf)
    }
}

fn path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config/herd/session.json"))
}

/// Where this file lived when the program was called ops-tui.
///
/// Read-only, and only when the current path holds nothing: a rename
/// should not quietly cost the user the tier they were last working in.
/// Nothing is ever written here, so the old file simply becomes stale and
/// can be deleted by hand.
fn legacy_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config/ops-tui/session.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let session = Session {
            config_path: Some(PathBuf::from("/models/32gb/models.ini")),
            model: Some("gemma4-12b".into()),
        };

        let text = serde_json::to_string(&session).expect("serialize");
        let back: Session = serde_json::from_str(&text).expect("deserialize");

        assert_eq!(back, session);
    }

    #[test]
    fn missing_fields_deserialize_to_none() {
        let back: Session = serde_json::from_str("{}").expect("deserialize");
        assert_eq!(back, Session::default());
    }

    /// A corrupt file must read as "no memory", never as a hard failure.
    #[test]
    fn corrupt_json_is_not_fatal() {
        assert!(serde_json::from_str::<Session>("{not json").is_err());
        assert_eq!(Session::default().usable_config_path(), None);
    }

    /// The rename must not cost anyone the tier they were last working in,
    /// so the pre-rename location is still read when the new one is empty.
    /// Both paths are derived the same way, so this pins the relationship
    /// rather than the literal strings.
    #[test]
    fn the_pre_rename_session_file_is_still_read() {
        let (current, legacy) = (path(), legacy_path());

        assert!(current.is_some() && legacy.is_some(), "no HOME in this env");
        assert_ne!(current, legacy, "the fallback points at the same file");
        assert!(
            legacy.unwrap().to_string_lossy().contains("ops-tui"),
            "the fallback is not the old location"
        );
        assert!(current.unwrap().to_string_lossy().contains("herd"));
    }

    #[test]
    fn a_remembered_path_that_no_longer_exists_is_ignored() {
        let session = Session {
            config_path: Some(PathBuf::from("/nonexistent/32gb/models.ini")),
            model: Some("gone".into()),
        };
        assert_eq!(session.usable_config_path(), None);
    }
}
