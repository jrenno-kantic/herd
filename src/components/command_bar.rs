use crate::{app::App, theme::Theme};
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn view(app: &App) -> Paragraph<'static> {
    Paragraph::new(format!(":{}", app.command_input))
        .style(Theme::command())
        .block(
            Block::default()
                .title("Command")
                .borders(Borders::ALL)
                .border_style(Theme::border()),
        )
}
