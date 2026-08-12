use ratatui::style::{Color, Modifier, Style};

pub struct Theme;

impl Theme {
    pub fn background() -> Style {
        Style::default().fg(Color::White).bg(Color::Black)
    }

    pub fn selected() -> Style {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD)
    }

    pub fn normal() -> Style {
        Style::default().fg(Color::White).bg(Color::Black)
    }

    pub fn logs() -> Style {
        Style::default().fg(Color::Gray).bg(Color::Black)
    }

    pub fn command() -> Style {
        Style::default().fg(Color::Yellow).bg(Color::Black)
    }

    pub fn border() -> Style {
        Style::default().fg(Color::DarkGray).bg(Color::Black)
    }

    pub fn status_ready() -> Style {
        Style::default()
            .fg(Color::Green)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD)
    }

    pub fn status_starting() -> Style {
        Style::default()
            .fg(Color::Yellow)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD)
    }

    /// The favourite star. Gold, and bold so it still reads on a terminal
    /// whose yellow is washed out.
    pub fn favorite() -> Style {
        Style::default()
            .fg(Color::Yellow)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD)
    }

    pub fn status_error() -> Style {
        Style::default()
            .fg(Color::Red)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD)
    }
}
