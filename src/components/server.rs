//! The Server screen: the lifecycle state, what is serving it, and the
//! tail of the process output.

use crate::{
    app::{App, Screen},
    keys,
    services::llama::ServerState,
    theme::Theme,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

const TAIL_LINES: usize = 12;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(10), Constraint::Min(3)])
        .split(area);

    frame.render_widget(summary(app), chunks[0]);
    frame.render_widget(tail(app), chunks[1]);
}

fn summary(app: &App) -> Paragraph<'static> {
    let server = &app.llama.server;
    let mut state_line = vec![
        Span::styled("  state     ", Theme::logs()),
        Span::styled(state_glyph(&server.state), state_style(&server.state)),
        Span::styled(server.state.label(), state_style(&server.state)),
    ];
    if let Some(phase) = server.phase.label() {
        let style = if server.is_degraded() {
            Theme::status_error()
        } else {
            Theme::status_starting()
        };
        state_line.push(Span::styled(format!("  {phase}"), style));
    }

    let mut lines = vec![
        Line::from(state_line),
        field("model", server.model.clone().unwrap_or_else(|| "-".into())),
        field("mode", server.mode.label().to_string()),
        field(
            "endpoint",
            server.endpoint.clone().unwrap_or_else(|| "-".into()),
        ),
        field("uptime", uptime(app)),
    ];

    lines.push(Line::from(""));
    lines.push(Line::styled(
        format!("  {}", keys::screen_hint(Screen::Server)),
        Theme::logs(),
    ));

    Paragraph::new(lines).block(block(" Server "))
}

fn field(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<12}"), Theme::logs()),
        Span::styled(value, Theme::normal()),
    ])
}

fn uptime(app: &App) -> String {
    match app.llama.server.uptime_secs() {
        None => "-".to_string(),
        Some(total) => {
            let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
            if h > 0 {
                format!("{h}h{m:02}m{s:02}s")
            } else if m > 0 {
                format!("{m}m{s:02}s")
            } else {
                format!("{s}s")
            }
        }
    }
}

fn state_glyph(state: &ServerState) -> &'static str {
    match state {
        ServerState::Off => "○ ",
        ServerState::Starting | ServerState::Stopping => "◐ ",
        ServerState::Serving => "● ",
        ServerState::Error(_) => "✖ ",
    }
}

pub fn state_style(state: &ServerState) -> ratatui::style::Style {
    match state {
        ServerState::Serving => Theme::status_ready(),
        ServerState::Starting | ServerState::Stopping => Theme::status_starting(),
        ServerState::Error(_) => Theme::status_error(),
        ServerState::Off => Theme::logs(),
    }
}

fn tail(app: &App) -> Paragraph<'static> {
    let start = app.logs.len().saturating_sub(TAIL_LINES);
    let text = app
        .logs
        .iter()
        .skip(start)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");

    Paragraph::new(text)
        .style(Theme::logs())
        .block(block(" recent output "))
}

fn block(title: &'static str) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Theme::border())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use std::path::PathBuf;

    fn app() -> App {
        App::with_config_path(PathBuf::from("/nonexistent/models.ini"))
    }

    #[test]
    fn uptime_reads_as_a_dash_when_nothing_runs() {
        assert_eq!(uptime(&app()), "-");
    }

    #[test]
    fn every_state_has_a_distinct_glyph() {
        let glyphs = [
            state_glyph(&ServerState::Off),
            state_glyph(&ServerState::Starting),
            state_glyph(&ServerState::Serving),
            state_glyph(&ServerState::Error("x".into())),
        ];
        // Starting and Stopping deliberately share the "in transition"
        // glyph; the four sampled here must all differ.
        for (i, a) in glyphs.iter().enumerate() {
            for b in glyphs.iter().skip(i + 1) {
                assert_ne!(a, b);
            }
        }
    }
}
