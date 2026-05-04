use ratatui::style::{Color, Style};

pub struct Theme;

impl Theme {
    pub fn selected() -> Style {
        Style::default().fg(Color::Black).bg(Color::Green)
    }

    pub fn normal() -> Style {
        Style::default().fg(Color::White)
    }
}