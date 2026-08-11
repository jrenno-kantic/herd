//! Session-only overrides for `[server]` / `[*]` / per-model options.
//!
//! Deliberately **never written to disk**: `models.ini` is hand-maintained
//! and heavily commented, and no ini round-tripper preserves comment
//! placement reliably. Edits live for the lifetime of the process and are
//! discarded on quit.
//!
//! They need no new precedence rule either. An override is exactly a CLI
//! override — "last write wins, keep original position" — so it is emitted
//! as extra argv and consumed by the existing final step of
//! `build_model_args`:
//!
//! ```text
//! [server] -> [*] -> [model] -> overrides -> explicit CLI args
//! ```

use std::collections::BTreeMap;

/// Which layer an edited key belongs to. Global edits apply to every
/// model; per-model edits apply only to the named preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    /// `[server]` and `[*]` — anything that is not model-specific.
    Global,
    /// A single `[model]` section.
    Model,
}

#[derive(Debug, Clone, Default)]
pub struct Overrides {
    global: BTreeMap<String, String>,
    per_model: BTreeMap<String, BTreeMap<String, String>>,
}

impl Overrides {
    pub fn is_empty(&self) -> bool {
        self.global.is_empty() && self.per_model.values().all(BTreeMap::is_empty)
    }

    pub fn set(&mut self, scope: Scope, model: &str, key: &str, value: &str) {
        let slot = match scope {
            Scope::Global => &mut self.global,
            Scope::Model => self.per_model.entry(model.to_string()).or_default(),
        };
        slot.insert(key.to_string(), value.to_string());
    }

    pub fn clear(&mut self, scope: Scope, model: &str, key: &str) {
        match scope {
            Scope::Global => {
                self.global.remove(key);
            }
            Scope::Model => {
                if let Some(slot) = self.per_model.get_mut(model) {
                    slot.remove(key);
                }
            }
        }
    }

    pub fn clear_all(&mut self) {
        self.global.clear();
        self.per_model.clear();
    }

    pub fn get(&self, scope: Scope, model: &str, key: &str) -> Option<&str> {
        match scope {
            Scope::Global => self.global.get(key).map(String::as_str),
            Scope::Model => self
                .per_model
                .get(model)
                .and_then(|slot| slot.get(key))
                .map(String::as_str),
        }
    }

    pub fn count(&self) -> usize {
        self.global.len() + self.per_model.values().map(BTreeMap::len).sum::<usize>()
    }

    /// Flattens to argv for `build_model_args`'s CLI-override slot. Global
    /// entries come first so a per-model override of the same key wins,
    /// matching the ini precedence the user already knows.
    pub fn to_args(&self, model: &str) -> Vec<String> {
        let per_model = self.per_model.get(model);

        self.global
            .iter()
            .chain(per_model.into_iter().flatten())
            .flat_map(|(key, value)| flag_args(key, value))
            .collect()
    }
}

/// The boolean spellings a `models.ini` actually uses, as pairs.
///
/// `1`/`0` are deliberately absent: they are indistinguishable from a
/// numeric setting that happens to be small, and silently flipping
/// `gpu-layers = 0` to `1` because it looked boolean would be a genuinely
/// bad edit. Only words are treated as booleans.
const BOOLEANS: [(&str, &str); 3] = [("true", "false"), ("on", "off"), ("yes", "no")];

/// The opposite of a boolean value, or `None` if it is not one.
///
/// Keeps the spelling family and the capitalisation the file already uses
/// — `on` becomes `off`, not `false`, and `True` becomes `False` — because
/// an edit that rewrites unrelated conventions makes a diff against the
/// original ini unreadable.
pub fn toggled(value: &str) -> Option<String> {
    let trimmed = value.trim();

    let opposite = BOOLEANS.iter().find_map(|(yes, no)| {
        if trimmed.eq_ignore_ascii_case(yes) {
            Some(*no)
        } else if trimmed.eq_ignore_ascii_case(no) {
            Some(*yes)
        } else {
            None
        }
    })?;

    Some(match case_of(trimmed) {
        Case::Upper => opposite.to_ascii_uppercase(),
        Case::Capitalised => capitalised(opposite),
        Case::Lower => opposite.to_string(),
    })
}

enum Case {
    Lower,
    Capitalised,
    Upper,
}

fn case_of(word: &str) -> Case {
    let mut letters = word.chars().filter(|c| c.is_alphabetic());

    match letters.next() {
        Some(first) if first.is_uppercase() => {
            if letters.all(char::is_uppercase) {
                Case::Upper
            } else {
                Case::Capitalised
            }
        }
        _ => Case::Lower,
    }
}

fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Mirrors the ini value conventions: `true` emits a bare flag, `false`
/// removes it (the CLI-override parser understands a bare `--no-<flag>`
/// only as a distinct flag, so `false` is emitted as `--<key> false` and
/// left to llama-server, matching what typing it by hand would do).
fn flag_args(key: &str, value: &str) -> Vec<String> {
    let flag = match key {
        "model" => "-m".to_string(),
        "hf" => "-hf".to_string(),
        other => format!("--{other}"),
    };

    if value.eq_ignore_ascii_case("true") {
        vec![flag]
    } else {
        vec![flag, value.to_string()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_overrides_emit_nothing() {
        let overrides = Overrides::default();
        assert!(overrides.is_empty());
        assert!(overrides.to_args("gemma4-12b").is_empty());
    }

    #[test]
    fn global_and_model_overrides_both_reach_the_argv() {
        let mut overrides = Overrides::default();
        overrides.set(Scope::Global, "", "port", "8080");
        overrides.set(Scope::Model, "gemma4-12b", "ctx-size", "65536");

        assert_eq!(
            overrides.to_args("gemma4-12b"),
            vec!["--port", "8080", "--ctx-size", "65536"]
        );
    }

    /// A per-model override must be able to beat a global one. Since
    /// `build_model_args` applies argv left to right with "last write
    /// wins", global has to be emitted first.
    #[test]
    fn per_model_override_is_emitted_after_global() {
        let mut overrides = Overrides::default();
        overrides.set(Scope::Global, "", "ctx-size", "32768");
        overrides.set(Scope::Model, "gemma4-12b", "ctx-size", "65536");

        assert_eq!(
            overrides.to_args("gemma4-12b"),
            vec!["--ctx-size", "32768", "--ctx-size", "65536"]
        );
    }

    #[test]
    fn overrides_are_scoped_to_the_named_model() {
        let mut overrides = Overrides::default();
        overrides.set(Scope::Model, "gemma4-12b", "ctx-size", "65536");

        assert!(overrides.to_args("qwen3-coder").is_empty());
    }

    #[test]
    fn boolean_true_emits_a_bare_flag() {
        let mut overrides = Overrides::default();
        overrides.set(Scope::Global, "", "jinja", "true");

        assert_eq!(overrides.to_args("any"), vec!["--jinja"]);
    }

    #[test]
    fn short_flags_match_the_ini_special_cases() {
        let mut overrides = Overrides::default();
        overrides.set(Scope::Model, "m", "hf", "unsloth/x:Q4");

        assert_eq!(overrides.to_args("m"), vec!["-hf", "unsloth/x:Q4"]);
    }

    #[test]
    fn clearing_removes_only_the_targeted_key() {
        let mut overrides = Overrides::default();
        overrides.set(Scope::Global, "", "port", "8080");
        overrides.set(Scope::Global, "", "host", "127.0.0.1");
        overrides.clear(Scope::Global, "", "port");

        assert_eq!(overrides.get(Scope::Global, "", "port"), None);
        assert_eq!(overrides.get(Scope::Global, "", "host"), Some("127.0.0.1"));
        assert_eq!(overrides.count(), 1);
    }

    #[test]
    fn a_boolean_toggles_to_its_opposite() {
        assert_eq!(toggled("true").as_deref(), Some("false"));
        assert_eq!(toggled("false").as_deref(), Some("true"));
        assert_eq!(toggled("on").as_deref(), Some("off"));
        assert_eq!(toggled("off").as_deref(), Some("on"));
        assert_eq!(toggled("yes").as_deref(), Some("no"));
        assert_eq!(toggled("no").as_deref(), Some("yes"));
    }

    /// `on` must not become `false`: rewriting the file's own convention
    /// makes a diff against the original ini unreadable.
    #[test]
    fn toggling_keeps_the_spelling_family_and_the_case() {
        assert_eq!(toggled("ON").as_deref(), Some("OFF"));
        assert_eq!(toggled("True").as_deref(), Some("False"));
        assert_eq!(toggled("  on  ").as_deref(), Some("off"));
    }

    /// The important half of the rule: anything that is not plainly a
    /// boolean must fall through to the normal editor rather than be
    /// silently rewritten. `0`/`1` are numbers here, not booleans.
    #[test]
    fn a_non_boolean_is_never_toggled() {
        for value in ["0", "1", "32768", "", "auto", "0.7", "unsloth/x:Q4"] {
            assert_eq!(toggled(value), None, "{value:?} was treated as a boolean");
        }
    }

    #[test]
    fn clear_all_empties_every_scope() {
        let mut overrides = Overrides::default();
        overrides.set(Scope::Global, "", "port", "8080");
        overrides.set(Scope::Model, "m", "ctx-size", "1024");
        overrides.clear_all();

        assert!(overrides.is_empty());
        assert_eq!(overrides.count(), 0);
    }
}
