use crate::{app::App, theme::Theme};
use ratatui::widgets::Paragraph;

pub fn view(app: &App) -> Paragraph<'static> {
    let state = if app.running { "running" } else { "idle" };
    let focus = format!("{:?}", app.focus).to_lowercase();

    Paragraph::new(format!(
        "status: {} | focus: {} | : command, tab focus, up/down navigate, help commands, q quit",
        state, focus
    ))
    .style(Theme::logs())
}
