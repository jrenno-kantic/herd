//! The Test screen: send a chat completion to the running model and show
//! the reply with its latency and token stats.
//!
//! This is `data/scripts/test_call.sh` as a screen — same system prompt,
//! same default message, same non-streaming request — but the prompt is
//! editable and the result is measured rather than dumped as raw JSON.

use crate::{
    app::{App, Mode, Screen},
    components, keys,
    services::llama::api::ChatOutcome,
    theme::Theme,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(3)])
        .split(area);

    frame.render_widget(request(app, chunks[0].width), chunks[0]);
    frame.render_widget(response(app), chunks[1]);
}

fn request(app: &App, width: u16) -> Paragraph<'static> {
    let target = app
        .llama
        .test_target()
        .unwrap_or_else(|| "no model".to_string());

    let editing = app.mode == Mode::EditPrompt;
    let prompt = if editing {
        format!("{}▏", app.llama.edit_buffer)
    } else {
        app.llama.prompt.clone()
    };

    // Counting up rather than a static "waiting…": a generation against a
    // large model takes as long as it takes, and a motionless line is
    // indistinguishable from a probe that will never come back.
    let status = match app.llama.chat_elapsed() {
        Some(elapsed) => Span::styled(
            format!("  waiting for the model… {:.1}s", elapsed.as_secs_f64()),
            Theme::status_starting(),
        ),
        None => Span::styled(
            format!(
                "  {}",
                keys::screen_hint_within(Screen::Test, components::hint_width(width, true, 0))
            ),
            Theme::logs(),
        ),
    };

    let lines = vec![
        field("model", target),
        field("state", app.llama.server.state.label()),
        Line::from(""),
        Line::from(vec![
            Span::styled("  prompt    ", Theme::logs()),
            Span::styled(
                prompt,
                if editing {
                    Theme::selected()
                } else {
                    Theme::normal()
                },
            ),
        ]),
        Line::from(""),
        Line::from(vec![status]),
    ];

    Paragraph::new(lines).block(block(" Test "))
}

fn field(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<12}"), Theme::logs()),
        Span::styled(value, Theme::normal()),
    ])
}

fn response(app: &App) -> Paragraph<'static> {
    let lines: Vec<Line> = match &app.llama.chat {
        None if app.llama.chat_pending => vec![Line::styled("…", Theme::logs())],
        None => vec![Line::styled("no test run yet", Theme::logs())],
        Some(Err(error)) => vec![Line::styled(error.clone(), Theme::status_error())],
        Some(Ok(outcome)) => reply_lines(outcome),
    };

    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(block(" Response "))
}

fn reply_lines(outcome: &ChatOutcome) -> Vec<Line<'static>> {
    let mut lines = vec![Line::styled(stats(outcome), Theme::status_ready())];

    if let Some(timings) = timings(outcome) {
        lines.push(Line::styled(timings, Theme::logs()));
    }
    lines.push(Line::from(""));

    for line in outcome.reply.lines() {
        lines.push(Line::styled(line.to_string(), Theme::normal()));
    }

    lines
}

/// What the round trip cost, measured here: when it went out, how long it
/// took, and what came back.
fn stats(outcome: &ChatOutcome) -> String {
    let mut parts = vec![
        format!("sent {}", outcome.sent_at.format("%H:%M:%S")),
        format!("{:.2}s", outcome.latency.as_secs_f64()),
    ];

    if let (Some(prompt), Some(completion)) = (outcome.prompt_tokens, outcome.completion_tokens) {
        parts.push(format!("{prompt} in / {completion} out"));
    }
    if let Some(rate) = outcome.tokens_per_second {
        parts.push(format!("{rate:.1} tok/s"));
    }

    parts.join("  ·  ")
}

/// The server's own split of that time, when it offers one.
///
/// Absent from every server but llama.cpp, and degrades to nothing rather
/// than to zeroes — the difference between "the prompt took no time" and
/// "the server did not say" is the whole point of reading it
/// opportunistically. The `overhead` line is what is left after the
/// server's own accounting: transport, queueing, and anything the server
/// spent before it started counting.
fn timings(outcome: &ChatOutcome) -> Option<String> {
    let mut parts = Vec::new();

    if let Some(ms) = outcome.prompt_ms {
        parts.push(format!("prompt eval {:.2}s", ms / 1000.0));
    }
    if let Some(ms) = outcome.predicted_ms {
        parts.push(format!("generation {:.2}s", ms / 1000.0));
    }
    if parts.is_empty() {
        return None;
    }

    let server_ms = outcome.prompt_ms.unwrap_or(0.0) + outcome.predicted_ms.unwrap_or(0.0);
    let overhead = outcome.latency.as_secs_f64() - server_ms / 1000.0;
    if overhead > 0.0 {
        parts.push(format!("overhead {overhead:.2}s"));
    }

    Some(parts.join("  ·  "))
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

    fn outcome() -> ChatOutcome {
        ChatOutcome {
            reply: "Bonjour ! Comment puis-je vous aider ?".into(),
            ..ChatOutcome::sample()
        }
    }

    #[test]
    fn stats_line_reports_when_it_was_sent_and_what_it_cost() {
        assert_eq!(
            stats(&outcome()),
            "sent 14:32:07  ·  1.25s  ·  24 in / 12 out  ·  9.6 tok/s"
        );
    }

    /// A server that reports no usage block still gets a useful line:
    /// the send time and the latency are measured here and always available.
    #[test]
    fn stats_degrade_to_what_was_measured_locally() {
        let bare = ChatOutcome {
            prompt_tokens: None,
            completion_tokens: None,
            tokens_per_second: None,
            ..outcome()
        };
        assert_eq!(stats(&bare), "sent 14:32:07  ·  1.25s");
    }

    /// The server-side split, when llama.cpp offers one. `overhead` is
    /// what the round trip cost beyond the server's own accounting.
    #[test]
    fn timings_break_the_round_trip_into_its_parts() {
        let timed = ChatOutcome {
            prompt_ms: Some(310.0),
            predicted_ms: Some(890.0),
            ..outcome()
        };
        assert_eq!(
            timings(&timed).as_deref(),
            Some("prompt eval 0.31s  ·  generation 0.89s  ·  overhead 0.05s")
        );
    }

    /// Any other server sends no `timings`, and the line must vanish
    /// rather than report a round trip that was all overhead.
    #[test]
    fn a_server_without_timings_gets_no_breakdown() {
        assert_eq!(timings(&outcome()), None);
    }

    /// Server timings can exceed the locally measured latency by a hair
    /// (different clocks, different start points). A negative overhead is
    /// noise, not information.
    #[test]
    fn an_overhead_that_does_not_exist_is_not_reported() {
        let timed = ChatOutcome {
            prompt_ms: Some(300.0),
            predicted_ms: Some(1000.0),
            ..outcome()
        };
        let line = timings(&timed).expect("timings");
        assert!(!line.contains("overhead"), "{line}");
    }

    #[test]
    fn a_multiline_reply_keeps_its_lines() {
        let multi = ChatOutcome {
            reply: "first\nsecond".into(),
            ..outcome()
        };
        // stats line + blank + two reply lines
        assert_eq!(reply_lines(&multi).len(), 4);
    }

    /// With timings present the reply gains a line, and the reply itself
    /// must still be the last thing shown.
    #[test]
    fn the_breakdown_sits_above_the_reply() {
        let timed = ChatOutcome {
            reply: "hello".into(),
            prompt_ms: Some(100.0),
            predicted_ms: Some(200.0),
            ..outcome()
        };
        // stats + timings + blank + one reply line
        assert_eq!(reply_lines(&timed).len(), 4);
    }
}
