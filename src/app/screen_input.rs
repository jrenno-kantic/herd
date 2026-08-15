//! Screen-level input routing shared by the application reducer.
//!
//! Modes such as editing, confirmation, and the command bar are handled
//! by `App` before input reaches here. This module owns the browse-screen
//! boundary and the common list navigation contract, keeping the central
//! reducer about event/state transitions rather than menu wiring.

use super::{Action, App, Screen};
use crossterm::event::{KeyCode, KeyEvent};

pub(super) fn dispatch(app: &mut App, key: KeyEvent) -> Action {
    match app.screen {
        Screen::Models => app.handle_models_key(key),
        Screen::Hub => app.handle_hub_key(key),
        Screen::Server => app.handle_server_key(key),
        Screen::Router => app.handle_router_key(key),
        Screen::Test => app.handle_test_key(key),
        Screen::Stats => app.handle_stats_key(key),
        Screen::Settings => app.handle_settings_key(key),
        Screen::Logs => app.handle_logs_key(key),
    }
}

/// Where a standard movement key takes a cursor over `len` rows.
/// Returning `None` leaves non-navigation keys to the active screen.
pub(super) fn moved(cursor: usize, len: usize, key: KeyCode, page: usize) -> Option<usize> {
    let last = len.checked_sub(1)?;

    let next = match key {
        KeyCode::Down | KeyCode::Char('j') => cursor.saturating_add(1).min(last),
        KeyCode::Up | KeyCode::Char('k') => cursor.saturating_sub(1),
        KeyCode::PageDown => cursor.saturating_add(page).min(last),
        KeyCode::PageUp => cursor.saturating_sub(page),
        KeyCode::Home | KeyCode::Char('g') => 0,
        KeyCode::End | KeyCode::Char('G') => last,
        _ => return None,
    };

    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_clamps_and_empty_lists_do_not_move() {
        assert_eq!(moved(0, 0, KeyCode::Down, 10), None);
        assert_eq!(moved(2, 3, KeyCode::Down, 10), Some(2));
        assert_eq!(moved(1, 3, KeyCode::PageDown, 10), Some(2));
        assert_eq!(moved(1, 3, KeyCode::PageUp, 10), Some(0));
    }
}
