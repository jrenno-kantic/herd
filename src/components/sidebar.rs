use crate::{
    app::{App, Screen},
    theme::Theme,
};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};

const VALUE_WIDTH: usize = 15;

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
        format!(" arch  {}", app.system.architecture),
        Theme::logs(),
    ));
    lines.push(Line::styled(
        format!(
            " GPU   {}",
            crate::components::truncate(app.system.gpu.as_deref().unwrap_or("?"), VALUE_WIDTH)
        ),
        Theme::logs(),
    ));
    lines.push(Line::styled(
        format!(
            " free  {}",
            app.system
                .available_memory_gib
                .map(|gib| format!("{gib:.1} GiB"))
                .unwrap_or_else(|| "?".into())
        ),
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
            // The version rides in the frame rather than taking a line of
            // its own: it is wanted once, when someone is writing down
            // which build misbehaved, and never otherwise.
            .title(format!("HERD {}", crate::version::short()))
            .borders(Borders::ALL)
            .border_style(Theme::border()),
    )
}
