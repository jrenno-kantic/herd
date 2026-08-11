use crate::{
    app::{App, Screen},
    theme::Theme,
};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn view(app: &App) -> Paragraph<'static> {
    let mut lines: Vec<Line> = Screen::ALL
        .iter()
        .enumerate()
        .map(|(index, screen)| {
            let selected = *screen == app.screen;
            Line::styled(
                format!(
                    " {} {} {}",
                    if selected { "▸" } else { " " },
                    index + 1,
                    screen.label()
                ),
                if selected {
                    Theme::selected()
                } else {
                    Theme::normal()
                },
            )
        })
        .collect();

    lines.push(Line::from(""));
    lines.push(Line::styled(
        format!(" tier  {}", app.llama.tier_name().unwrap_or("-")),
        Theme::logs(),
    ));
    lines.push(Line::styled(
        format!(
            " RAM   {}",
            app.llama
                .ram_gib
                .map(|gib| format!("{gib} GiB"))
                .unwrap_or_else(|| "?".into())
        ),
        Theme::logs(),
    ));

    Paragraph::new(lines).block(
        Block::default()
            .title("HERD")
            .borders(Borders::ALL)
            .border_style(Theme::border()),
    )
}
