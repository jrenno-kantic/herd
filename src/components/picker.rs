//! Modal listing every `models.ini` available on this machine.
//!
//! Each row reports how many of its presets the current memory budget
//! cannot hold, in red — picking the 32gb tier on a 16 GiB machine should
//! say so before you launch something that will not fit.

use crate::{app::App, components::centered, theme::Theme};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let choices = app.llama.config_choices();
    let popup = centered(area, 78, (choices.len() as u16).saturating_add(7).min(20));

    let mut lines: Vec<Line> = vec![Line::styled(
        format!(
            "  budget {:.1} GiB usable of {} GiB installed",
            app.llama.budget().available_gib(),
            app.llama
                .ram_gib
                .map(|gib| gib.to_string())
                .unwrap_or_else(|| "?".into())
        ),
        Theme::logs(),
    )];

    if choices.is_empty() {
        lines.push(Line::styled(
            "  no models.ini found under ~/models",
            Theme::status_error(),
        ));
    }

    for (index, choice) in choices.iter().enumerate() {
        let selected = index == app.llama.picker_cursor;
        let active = choice.path == app.llama.config_path;

        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "{}{} {:<8} {:>3} presets  ",
                    if selected { "▸" } else { " " },
                    if active { "•" } else { " " },
                    choice.label,
                    choice.presets
                ),
                if selected {
                    Theme::selected()
                } else {
                    Theme::normal()
                },
            ),
            warning_span(choice.too_large),
        ]));

        lines.push(Line::styled(
            format!("     {}", choice.path.display()),
            Theme::logs(),
        ));
    }

    lines.push(Line::from(""));
    lines.push(Line::styled(
        "  enter select · up/down move · esc cancel",
        Theme::logs(),
    ));

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).style(Theme::normal()).block(
            Block::default()
                .title(" Select models.ini ")
                .borders(Borders::ALL)
                .border_style(Theme::border()),
        ),
        popup,
    );
}

fn warning_span(too_large: usize) -> Span<'static> {
    match too_large {
        0 => Span::styled("", Theme::normal()),
        n => Span::styled(format!("⚠ {n} exceed this machine"), Theme::status_error()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_within_budget_carries_no_warning() {
        assert_eq!(warning_span(0).content, "");
    }

    #[test]
    fn an_oversized_config_is_called_out() {
        assert_eq!(warning_span(6).content, "⚠ 6 exceed this machine");
    }
}
