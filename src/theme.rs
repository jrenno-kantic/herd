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
}
