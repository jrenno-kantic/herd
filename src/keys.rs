//! The keymap, as data.
//!
//! Bindings used to be written down twice: once as match arms in
//! `app.rs`'s `handle_*_key`, and once as a hand-written hint string in
//! each component. The two drifted — `T`, `r`, `X`, `Shift+Tab` and the
//! digit shortcuts were all handled but documented nowhere.
//!
//! This table is the single source the screen footers, the status bar and
//! the `?` overlay all read from. It is also checked against the
//! dispatcher: `every_key_that_does_something_is_documented` in `app.rs`
//! drives every key into every screen and fails if one changes the app
//! without appearing here.

use crate::app::Screen;
use crossterm::event::{KeyCode, KeyEvent};

/// One documented binding. Several keys per entry because `j` and `↓` are
/// one idea, not two.
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    /// Canonical key tokens, as produced by [`token`]. Read by the
    /// conformance test rather than by the UI, which only ever shows
    /// `label` — hence the allow.
    #[cfg_attr(not(test), allow(dead_code))]
    pub keys: &'static [&'static str],
    /// How the keys read on screen (`"j/↓"`).
    pub label: &'static str,
    /// Full description, for the `?` overlay.
    pub action: &'static str,
    /// Two or three words for the footer. A footer that runs past the
    /// edge of the pane is worse than a terse one: the last hint on the
    /// line simply disappears.
    pub short: &'static str,
    /// Whether it earns a place in that footer at all. Everything is
    /// listed in the overlay; only the load-bearing keys make the footer.
    pub brief: bool,
}

const fn brief(
    keys: &'static [&'static str],
    label: &'static str,
    short: &'static str,
    action: &'static str,
) -> Binding {
    Binding {
        keys,
        label,
        action,
        short,
        brief: true,
    }
}

const fn full(keys: &'static [&'static str], label: &'static str, action: &'static str) -> Binding {
    Binding {
        keys,
        label,
        action,
        short: action,
        brief: false,
    }
}

/// Keys handled in `Browse` on every screen.
pub const GLOBAL: &[Binding] = &[
    brief(&["tab", "right"], "tab/→", "screen", "next screen"),
    full(&["shift+tab", "left"], "⇧tab/←", "previous screen"),
    full(
        &["1", "2", "3", "4", "5", "6"],
        "1–6",
        "jump straight to a screen",
    ),
    brief(&["c"], "c", "config", "choose which models.ini to use"),
    brief(&[":"], ":", "command", "command bar"),
    brief(&["?"], "?", "help", "this help"),
    brief(
        &["q"],
        "q",
        "quit",
        "quit — asks first if work is in flight",
    ),
    full(&["Q"], "Q", "quit at once, abandoning anything in flight"),
];

/// Movement shared by the Models table and the Settings rows.
const MOVE: &[Binding] = &[
    brief(&["j", "down"], "j/↓", "move", "move down"),
    full(&["k", "up"], "k/↑", "move up"),
    full(&["pgdn", "pgup"], "pgdn/pgup", "move by a page"),
    full(&["g", "home"], "g/home", "first row"),
    full(&["G", "end"], "G/end", "last row"),
];

/// The same keys on the Logs screen, described the way a scrollback reads
/// rather than as rows: the buffer grows downwards and the resting
/// position is the end of it, so "down" means "towards the newest line".
const LOGS: &[Binding] = &[
    brief(&["k", "up"], "k/↑", "back", "scroll back"),
    brief(
        &["j", "down"],
        "j/↓",
        "forward",
        "scroll towards the newest",
    ),
    full(&["pgdn", "pgup"], "pgdn/pgup", "scroll by a page"),
    full(&["g", "home"], "g/home", "oldest line kept"),
    brief(&["G", "end"], "G/end", "newest", "back to the newest line"),
];

const MODELS: &[Binding] = &[
    brief(
        &["enter"],
        "enter",
        "launch",
        "launch the highlighted preset",
    ),
    brief(
        &["s"],
        "s",
        "stop",
        "stop the server, or clear a failed launch",
    ),
    brief(
        &["d"],
        "d",
        "download",
        "download the highlighted preset without launching it",
    ),
    brief(&["/"], "/", "filter", "filter presets by name"),
    brief(&["t", "T"], "t/T", "tier", "next/previous tier"),
    full(&["r"], "r", "re-read models.ini from disk"),
];

const SERVER: &[Binding] = &[
    brief(
        &["enter"],
        "enter",
        "launch",
        "launch the selected preset — refused when it is the one already serving",
    ),
    brief(&["s"], "s", "stop", "stop the server"),
    brief(&["p"], "p", "ping", "ping the running model"),
];

const TEST: &[Binding] = &[
    brief(&["enter"], "enter", "send", "send the prompt"),
    brief(&["e"], "e", "edit prompt", "edit the prompt"),
    brief(&["r"], "r", "reset", "reset the prompt and the result"),
];

const STATS: &[Binding] = &[
    brief(
        &["+", "=", "-", "_"],
        "+/-",
        "adjust",
        "adjust the memory reservation",
    ),
    brief(&["r"], "r", "reset", "reset it to the default"),
];

const SETTINGS: &[Binding] = &[
    brief(
        &["enter"],
        "enter",
        "edit/toggle",
        "edit the highlighted setting — or flip it, when it is true/false or on/off",
    ),
    brief(&["x"], "x", "clear", "clear this override"),
    brief(&["X"], "X", "clear all", "clear every override"),
];

/// Bindings specific to `screen`, movement keys first where the screen
/// has a list to move through.
pub fn for_screen(screen: Screen) -> Vec<Binding> {
    let (moves, own): (bool, &[Binding]) = match screen {
        Screen::Models => (true, MODELS),
        Screen::Server => (false, SERVER),
        Screen::Test => (false, TEST),
        Screen::Stats => (false, STATS),
        Screen::Settings => (true, SETTINGS),
        Screen::Logs => (false, LOGS),
    };

    let mut bindings = Vec::new();
    if moves {
        bindings.extend_from_slice(MOVE);
    }
    bindings.extend_from_slice(own);
    bindings
}

/// True when `token` is documented for `screen`, globally or otherwise.
/// Exists for the conformance test in `app.rs`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn documents(screen: Screen, token: &str) -> bool {
    GLOBAL
        .iter()
        .chain(for_screen(screen).iter())
        .any(|binding| binding.keys.contains(&token))
}

/// One-line footer for a screen: its own brief bindings only, since the
/// status bar already carries the global ones.
pub fn screen_hint(screen: Screen) -> String {
    join(for_screen(screen).iter().filter(|binding| binding.brief))
}

pub fn global_hint() -> String {
    join(GLOBAL.iter().filter(|binding| binding.brief))
}

fn join<'a>(bindings: impl Iterator<Item = &'a Binding>) -> String {
    bindings
        .map(|binding| format!("{} {}", binding.label, binding.short))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Canonical name for a key, so a binding table entry and a `KeyEvent`
/// can be compared. `None` for keys no binding could name.
#[cfg_attr(not(test), allow(dead_code))]
pub fn token(key: KeyEvent) -> Option<String> {
    let name = match key.code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Esc => "esc".to_string(),
        // Crossterm already reports Shift+Tab as its own code.
        KeyCode::Tab => "tab".to_string(),
        KeyCode::BackTab => "shift+tab".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::PageUp => "pgup".to_string(),
        KeyCode::PageDown => "pgdn".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::Backspace => "backspace".to_string(),
        _ => return None,
    };
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_screen_documents_something() {
        for screen in Screen::ALL {
            assert!(
                !for_screen(screen).is_empty(),
                "{screen:?} has no documented bindings"
            );
        }
    }

    /// A screen-local key that reuses a global one would be shadowed by
    /// `handle_browse_key`, which matches the globals first — the binding
    /// would be listed and never fire.
    #[test]
    fn no_screen_binding_shadows_a_global_one() {
        for screen in Screen::ALL {
            for binding in for_screen(screen) {
                for key in binding.keys {
                    assert!(
                        !GLOBAL.iter().any(|global| global.keys.contains(key)),
                        "{screen:?} binds {key:?}, which is already global"
                    );
                }
            }
        }
    }

    #[test]
    fn a_screen_never_binds_the_same_key_twice() {
        for screen in Screen::ALL {
            let mut seen: Vec<&str> = Vec::new();
            for binding in for_screen(screen) {
                for key in binding.keys {
                    assert!(!seen.contains(key), "{screen:?} binds {key:?} twice");
                    seen.push(key);
                }
            }
        }
    }

    #[test]
    fn hints_name_the_keys_they_describe() {
        let hint = screen_hint(Screen::Models);
        assert!(hint.contains("enter launch"), "{hint}");
        assert!(hint.contains("t/T tier"), "{hint}");
        assert!(global_hint().contains("q quit"));
    }

    /// A footer wider than the pane loses its last hints off the edge.
    /// The narrowest pane in the layout is the main area beside the
    /// 24-column sidebar on a 100-column terminal.
    #[test]
    fn footers_fit_the_pane_they_are_drawn_in() {
        const BUDGET: usize = 100 - 24 - 2;

        for screen in Screen::ALL {
            let hint = screen_hint(screen);
            assert!(
                hint.chars().count() <= BUDGET,
                "{screen:?} footer is {} chars: {hint}",
                hint.chars().count()
            );
        }
        assert!(global_hint().chars().count() <= 100);
    }

    #[test]
    fn tokens_round_trip_through_the_table() {
        use crossterm::event::KeyModifiers;

        let enter = token(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).expect("enter");
        assert!(documents(Screen::Models, &enter));
        assert!(!documents(Screen::Logs, &enter));

        let back = token(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)).expect("backtab");
        assert!(documents(Screen::Logs, &back), "globals apply everywhere");
    }
}
