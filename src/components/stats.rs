//! The Stats screen: what this serving session has done, and how much
//! memory the machine is willing to give it.

use crate::{
    app::{App, Screen},
    components, keys,
    services::llama::memory,
    theme::Theme,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(5)])
        .split(area);

    frame.render_widget(session(app), chunks[0]);
    frame.render_widget(memory_budget(app, chunks[1].width), chunks[1]);
}

fn session(app: &App) -> Paragraph<'static> {
    let stats = &app.llama.stats;
    let server = &app.llama.server;

    let uptime = server
        .uptime_secs()
        .map(format_duration)
        .unwrap_or_else(|| "-".into());

    let lines = vec![
        field("model", server.model.clone().unwrap_or_else(|| "-".into())),
        field("state", server.state.label()),
        field("started", stats.started_label()),
        field("uptime", uptime),
        Line::from(""),
        field(
            "requests",
            match stats.failures {
                0 => stats.probes.to_string(),
                failures => format!("{} ({failures} failed)", stats.probes),
            },
        ),
        field("tokens in", stats.prompt_tokens.to_string()),
        field("tokens out", stats.completion_tokens.to_string()),
        field("throughput", throughput(app)),
    ];

    Paragraph::new(lines).block(block(" Session "))
}

fn throughput(app: &App) -> String {
    let stats = &app.llama.stats;

    let mut parts = Vec::new();
    if let Some(average) = stats.average_rate() {
        parts.push(format!("{average:.1} tok/s avg"));
    }
    if let Some(last) = stats.last_rate {
        parts.push(format!("{last:.1} last"));
    }
    if let Some(best) = stats.best_rate {
        parts.push(format!("{best:.1} best"));
    }

    if parts.is_empty() {
        "-  (run a test on screen 3)".to_string()
    } else {
        parts.join("  ·  ")
    }
}

fn memory_budget(app: &App, width: u16) -> Paragraph<'static> {
    let budget = app.llama.budget();
    let risky = budget.is_risky();

    let total = match app.llama.ram_gib {
        Some(gib) => format!("{gib} GiB"),
        None => "unknown".to_string(),
    };

    let mut lines = vec![
        field("installed", total),
        Line::from(vec![
            Span::styled("  reserved  ", Theme::logs()),
            Span::styled(
                format!(
                    "{:.0}%  ({:.1} GiB)",
                    budget.reserved_ratio * 100.0,
                    budget.reserved_gib()
                ),
                if risky {
                    Theme::status_error()
                } else {
                    Theme::normal()
                },
            ),
        ]),
        field("for models", format!("{:.1} GiB", budget.available_gib())),
        Line::from(""),
    ];

    if risky {
        lines.push(Line::styled(
            "  ⚠ CAUTION  less is reserved than the system default.",
            Theme::status_error(),
        ));
        lines.push(Line::styled(
            "    The OS may swap, stall, or kill the server under load.",
            Theme::status_error(),
        ));
        lines.push(Line::from(""));
    }

    lines.push(Line::styled(
        format!(
            "  This only changes how herd judges whether a preset fits (default {:.0}%).",
            memory::DEFAULT_RESERVED_RATIO * 100.0
        ),
        Theme::logs(),
    ));
    lines.push(Line::styled(
        "  It does not change any system setting. On macOS the real GPU limit is",
        Theme::logs(),
    ));
    lines.push(Line::styled(
        "  sudo sysctl iogpu.wired_limit_mb=<MB> — run that yourself if you mean it.",
        Theme::logs(),
    ));
    lines.push(Line::from(""));
    lines.push(Line::styled(
        format!(
            "  {}",
            keys::screen_hint_within(Screen::Stats, components::hint_width(width, true, 0))
        ),
        Theme::logs(),
    ));

    Paragraph::new(lines).block(block(" Memory budget "))
}

fn field(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<12}"), Theme::logs()),
        Span::styled(value, Theme::normal()),
    ])
}

fn format_duration(total: u64) -> String {
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
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
    use crate::services::llama::api::ChatOutcome;
    use crate::{event::UiEvent, services::llama::memory::DEFAULT_RESERVED_RATIO};
    use std::path::PathBuf;
    use std::time::Duration;

    fn app() -> App {
        App::with_config_path(PathBuf::from("/nonexistent/models.ini"))
    }

    fn probe(app: &mut App, completion: u64, rate: f64, seconds: u64) {
        app.update(UiEvent::ChatResult(Box::new(Ok(ChatOutcome {
            latency: Duration::from_secs(seconds),
            prompt_tokens: Some(10),
            completion_tokens: Some(completion),
            tokens_per_second: Some(rate),
            ..ChatOutcome::sample()
        }))));
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(format_duration(9), "9s");
        assert_eq!(format_duration(70), "1m10s");
        assert_eq!(format_duration(3725), "1h02m05s");
    }

    #[test]
    fn throughput_points_at_the_test_screen_before_any_probe() {
        assert!(throughput(&app()).contains("screen 3"));
    }

    /// The average must come from totals, not from averaging the
    /// per-request rates: a slow first request would otherwise be weighted
    /// the same as a fast later one.
    #[test]
    fn throughput_averages_over_the_whole_session() {
        let mut app = app();
        probe(&mut app, 100, 50.0, 4); // 100 tokens in 4s
        probe(&mut app, 100, 25.0, 1); // 100 tokens in 1s

        // 200 tokens over 5s = 40 tok/s, not (50+25)/2.
        let text = throughput(&app);
        assert!(text.contains("40.0 tok/s avg"), "{text}");
        assert!(text.contains("25.0 last"), "{text}");
        assert!(text.contains("50.0 best"), "{text}");
    }

    #[test]
    fn failures_are_counted_separately() {
        let mut app = app();
        probe(&mut app, 10, 5.0, 1);
        app.update(UiEvent::ChatResult(Box::new(Err("boom".into()))));

        assert_eq!(app.llama.stats.probes, 1);
        assert_eq!(app.llama.stats.failures, 1);
    }

    #[test]
    fn the_default_budget_is_not_flagged_risky() {
        let app = app();
        assert_eq!(app.llama.reserved_ratio, DEFAULT_RESERVED_RATIO);
        assert!(!app.llama.budget().is_risky());
    }
}
