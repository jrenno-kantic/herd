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
        &["1", "2", "3", "4", "5", "6", "7", "8"],
        "1–8",
        "jump straight to a screen",
    ),
    brief(&["c"], "c", "config", "choose which models.ini to use"),
    brief(&[":"], ":", "command", "command bar"),
    brief(&["?"], "?", "help", "this help"),
    brief(&["q"], "q", "quit", "quit — asks if a server is active"),
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
    // Last of the brief bindings deliberately: the drop order runs from
    // the end, and of these keys a star is the one whose absence from a
    // narrow footer costs least.
    brief(
        &["f"],
        "f",
        "star",
        "star this preset, or take the star off",
    ),
    // Not in the footer, which is already two characters off its budget at
    // 100 columns — the argv preview carries this hint on its own border
    // instead, where the thing being copied is. Hence a terse `short`
    // despite `brief: false`: that is what the border shows.
    Binding {
        keys: &["y"],
        label: "y",
        action: "copy the launch command to the clipboard",
        short: "copy",
        brief: false,
    },
    // Not in the footer: the preview's own border carries it, and only
    // when there is something below the fold — a key advertised where it
    // would do nothing is worse than one nobody mentioned.
    Binding {
        keys: &["J", "K"],
        label: "J/K",
        action: "scroll the argv preview",
        short: "scroll",
        brief: false,
    },
    full(&["r"], "r", "re-read models.ini from disk"),
];

/// The cache, as against the tier. `D` is uppercase because it is the one
/// destructive key in the program and `d` next door means *download* —
/// see `App::handle_hub_key`.
const HUB: &[Binding] = &[
    brief(
        &["y"],
        "y",
        "copy preset",
        "copy a models.ini stanza for this model",
    ),
    brief(
        &["enter"],
        "enter",
        "show",
        "show this model's preset on the Models screen",
    ),
    brief(&["r"], "r", "refresh", "ask llama.cpp again what it has"),
    // Last of the brief bindings deliberately: hints drop from the end,
    // and a footer too narrow to name a destructive key is a footer that
    // should be naming the others.
    brief(
        &["D"],
        "D",
        "delete",
        "delete this model from the cache — asks first",
    ),
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

const ROUTER: &[Binding] = &[
    brief(
        &["+", "=", "-", "_"],
        "+/-",
        "adjust",
        "adjust the highlighted setting",
    ),
    brief(
        &["enter"],
        "enter",
        "start",
        "start the router with these settings",
    ),
    brief(&["s"], "s", "stop", "stop the router"),
    brief(&["r"], "r", "reset", "reset the settings to the defaults"),
    Binding {
        keys: &["y"],
        label: "y",
        action: "copy the router command to the clipboard",
        short: "copy",
        brief: false,
    },
    // Not in the footer: the preview's own border carries it, and only
    // when there is something below the fold — a key advertised where it
    // would do nothing is worse than one nobody mentioned.
    Binding {
        keys: &["J", "K"],
        label: "J/K",
        action: "scroll the argv preview",
        short: "scroll",
        brief: false,
    },
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
    brief(
        &["m"],
        "m",
        "mono-focus",
        "switch the [mono-focus] profile on or off for this preset",
    ),
    brief(&["x"], "x", "clear", "clear this override"),
    brief(&["X"], "X", "clear all", "clear every override"),
];

/// Bindings specific to `screen`, movement keys first where the screen
/// has a list to move through.
pub fn for_screen(screen: Screen) -> Vec<Binding> {
    let (moves, own): (bool, &[Binding]) = match screen {
        Screen::Models => (true, MODELS),
        Screen::Hub => (true, HUB),
        Screen::Server => (false, SERVER),
        // The shared movement keys, even though there are only two rows:
        // they all reach `moved`, so they all have to be documented.
        Screen::Router => (true, ROUTER),
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
/// status bar already carries the global ones. Every caller draws into a
/// pane and so goes through [`screen_hint_within`]; this is the unfitted
/// form the tests measure against.
#[cfg_attr(not(test), allow(dead_code))]
pub fn screen_hint(screen: Screen) -> String {
    join(for_screen(screen).iter().filter(|binding| binding.brief))
}

/// The same footer, fitted to the width it will be drawn in.
///
/// The hints outgrew the pane the moment a screen gained a seventh key:
/// at 100 columns the Models footer had 74 to work with and wanted 76, and
/// a footer that is too long does not wrap — its last hints simply
/// disappear off the right edge, which is the same dishonesty the table's
/// `Columns::for_width` exists to avoid. So hints are dropped from the
/// end, least load-bearing first (the table is ordered that way), and the
/// line **says that it dropped some**: `… ? more` points at the overlay
/// that has all of them. A hint that vanished silently would read as a key
/// that does not exist.
pub fn screen_hint_within(screen: Screen, width: usize) -> String {
    // An ellipsis, not "? more": the marker is subtracted from the room
    // the real hints get, and at 100 columns the six characters of the
    // longer form cost the Models footer its tier key — a hint worth more
    // than the words explaining where the rest went. `?` is named in the
    // status bar on every screen.
    const MORE: &str = "…";

    let bindings: Vec<Binding> = for_screen(screen)
        .into_iter()
        .filter(|binding| binding.brief)
        .collect();

    let full = join(bindings.iter());
    if full.chars().count() <= width {
        return full;
    }

    // Take from the front while what is taken, plus the marker saying
    // there is more, still fits.
    let mut kept = 0;
    while kept < bindings.len() {
        let candidate = join(bindings[..=kept].iter());
        // Counted in characters, not bytes: the separator's `·` and the
        // ellipsis are multi-byte, and `len()` here would reserve seven
        // columns for four — enough to cost the Models footer a hint.
        let marker = SEPARATOR.chars().count() + MORE.chars().count();
        if candidate.chars().count() + marker > width {
            break;
        }
        kept += 1;
    }

    if kept == 0 {
        // Narrower than one hint plus the marker: say only where the rest
        // is, rather than half a word.
        return MORE.chars().take(width).collect();
    }

    format!("{}{SEPARATOR}{MORE}", join(bindings[..kept].iter()))
}

/// The footer form of one binding (`"y copy"`), for a screen that shows a
/// hint somewhere other than in its footer. Looked up rather than written
/// out a second time, which is the drift this table exists to prevent.
pub fn hint_for(screen: Screen, key: &str) -> Option<String> {
    for_screen(screen)
        .iter()
        .find(|binding| binding.keys.contains(&key))
        .map(|binding| format!("{} {}", binding.label, binding.short))
}

pub fn global_hint() -> String {
    join(GLOBAL.iter().filter(|binding| binding.brief))
}

const SEPARATOR: &str = " · ";

fn join<'a>(bindings: impl Iterator<Item = &'a Binding>) -> String {
    bindings
        .map(|binding| format!("{} {}", binding.label, binding.short))
        .collect::<Vec<_>>()
        .join(SEPARATOR)
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

    /// A footer wider than its pane loses its last hints off the edge,
    /// silently. `screen_hint_within` is what stops that, and this is the
    /// assertion that keeps it honest at every width a pane can have —
    /// the same shape as `no_width_produces_a_row_that_overflows` for the
    /// Models table.
    #[test]
    fn a_footer_never_overflows_the_pane_it_is_drawn_in() {
        for screen in Screen::ALL {
            for width in 0..=140 {
                let hint = screen_hint_within(screen, width);
                assert!(
                    hint.chars().count() <= width,
                    "{screen:?} at {width}: {} chars — {hint}",
                    hint.chars().count()
                );
            }
        }
        assert!(global_hint().chars().count() <= 100);
    }

    /// Dropping hints is allowed; dropping them without saying so is not.
    /// A hint that vanished silently reads as a key that does not exist,
    /// which is the whole failure this replaced.
    #[test]
    fn a_shortened_footer_says_there_is_more() {
        let full = screen_hint(Screen::Models);
        let width = full.chars().count() - 1;
        let fitted = screen_hint_within(Screen::Models, width);

        assert_ne!(fitted, full, "nothing was dropped at all");
        assert!(fitted.ends_with('…'), "{fitted}");
        // Even with no room for a single hint, it still says so.
        assert_eq!(screen_hint_within(Screen::Models, 3), "…");
    }

    /// ...and a pane with room for everything shows everything.
    #[test]
    fn a_wide_pane_keeps_every_hint() {
        for screen in Screen::ALL {
            let full = screen_hint(screen);
            assert_eq!(screen_hint_within(screen, 200), full, "{screen:?}");
            assert!(!full.contains('…'));
        }
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
