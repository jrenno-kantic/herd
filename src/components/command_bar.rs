//! The `:` bar, and the one thing it has to say about itself.
//!
//! A blank prompt with a border reading "Command" tells you that commands
//! exist and nothing about which. The hint on the border is where that is
//! answered — the same trick the argv preview uses for `y copy`, and for
//! the same reason: the pointer belongs on the thing it acts on.

use crate::{app::App, commands, theme::Theme};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn view(app: &App) -> Paragraph<'static> {
    Paragraph::new(format!(":{}", app.command_input))
        .style(Theme::command())
        .block(
            Block::default()
                .title("Command")
                .borders(Borders::ALL)
                .border_style(Theme::border())
                .title_top(Line::styled(hint(app), Theme::border()).right_aligned()),
        )
}

/// What the border says, on the right.
///
/// While a line is being typed it names the command that would run, which
/// is the moment a typo is cheapest to notice — `:lauch` reads as
/// "unknown", before Enter rather than after it. Otherwise it points at
/// the listing.
fn hint(app: &App) -> String {
    let typed = app.command_input.trim();

    if typed.is_empty() {
        return " :help lists the commands ".to_string();
    }

    match commands::find(typed) {
        Some(command) => format!(" {} ", command.summary),
        None => " unknown — :help lists the commands ".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn app() -> App {
        App::with_config_path(PathBuf::from("/nonexistent/models.ini"))
    }

    #[test]
    fn an_empty_bar_points_at_the_listing() {
        assert!(hint(&app()).contains(":help"));
    }

    /// A recognised command explains itself as it is typed, arguments and
    /// all — the summary is for the command, not for the exact line.
    #[test]
    fn a_known_command_is_described_while_it_is_typed() {
        let mut app = app();
        app.command_input = "launch gemma4-12b".into();

        assert!(hint(&app).contains("hot-swapping"), "{}", hint(&app));
    }

    /// The failure worth catching before Enter rather than after it.
    #[test]
    fn a_typo_says_so_before_it_is_submitted() {
        let mut app = app();
        app.command_input = "lauch".into();

        assert!(hint(&app).starts_with(" unknown"), "{}", hint(&app));
    }
}
