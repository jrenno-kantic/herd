use crate::{app::App, theme::Theme};
use ratatui::widgets::{Block, Borders, Paragraph};

const VISIBLE_LINES: usize = 200;

pub fn view(app: &App) -> Paragraph<'static> {
    let start = app.logs.len().saturating_sub(VISIBLE_LINES);
    let text = app
        .logs
        .iter()
        .skip(start)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");

    Paragraph::new(text).style(Theme::logs()).block(
        Block::default()
            .title("Logs")
            .borders(Borders::ALL)
            .border_style(Theme::border()),
    )
}
