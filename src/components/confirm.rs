//! Modal shown when a launch is worth a second look: the port is already
//! bound by a process herd did not start, or the preset is estimated to
//! need more memory than the machine can give it.
//!
//! For a port conflict it offers to launch anyway, not to kill the other
//! process: herd has no way to know what that process is, and
//! terminating something it never started is not a call it should make on
//! the user's behalf. For an oversized preset it states both numbers and
//! lets the user decide — the estimate is a heuristic, and the user may
//! well know better than it does.

use crate::{
    app::{App, Confirm},
    components::centered,
    services::llama::{LauncherMode, ServerState},
    theme::Theme,
};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// The exit prompt: which supervised server will stop, what else would be
/// abandoned, and how to proceed anyway.
///
/// Separate from the launch prompt because it answers a different question
/// and lists live state rather than a static reason.
pub fn render_quit(frame: &mut Frame, app: &App, area: Rect) {
    let lines = quit_lines(app);
    let popup = centered(area, 68, (lines.len() + 2) as u16);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).style(Theme::normal()).block(
            Block::default()
                .title(" Confirm quit ")
                .borders(Borders::ALL)
                .border_style(Theme::status_error()),
        ),
        popup,
    );
}

/// The prompt's text, apart from the drawing of it, so what it says can be
/// asserted on without a terminal — the same split as `about::lines`.
fn quit_lines(app: &App) -> Vec<Line<'static>> {
    let work = app.in_flight();
    let server_live = app.llama.server.state.is_live();
    let mut lines = vec![
        Line::from(""),
        Line::styled("  Quit HERD?".to_string(), Theme::normal()),
        Line::styled(format!("  {}", serving_summary(app)), Theme::status_ready()),
    ];

    if !work.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "  Quitting now would also abandon:".to_string(),
            Theme::status_error(),
        ));
        for item in &work {
            lines.push(Line::styled(format!("    · {item}"), Theme::normal()));
        }
        // Said only when there is a download, and said because it is
        // true: `hf` writes into a `.incomplete` blob and picks up from
        // it, so this one item on the list is the recoverable one. A
        // prompt that let the user believe six gigabytes were about to be
        // thrown away would be scaring them into the wrong answer.
        if app.llama.download.is_some() {
            lines.push(Line::styled(
                "    the download resumes from where it stopped".to_string(),
                Theme::logs(),
            ));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        if server_live {
            "  The supervised server will be stopped on exit."
        } else {
            "  No supervised server needs to be stopped on exit."
        }
        .to_string(),
        Theme::logs(),
    ));
    lines.push(Line::styled(
        "  Confirm quit?   [y] yes   [any other key] stay".to_string(),
        Theme::normal(),
    ));

    lines
}

/// Human-facing state for the quit decision.
///
/// Manual mode names the one model HERD supervises. Router mode deliberately
/// reports its configured resident limit rather than claiming an exact loaded
/// count: llama-server owns that list and does not send it with lifecycle
/// snapshots.
fn serving_summary(app: &App) -> String {
    let server = &app.llama.server;
    let model = crate::components::truncate(server.model.as_deref().unwrap_or("unknown model"), 18);

    match (&server.state, server.mode) {
        (ServerState::Starting, LauncherMode::Manual) => {
            format!("Model '{model}' is currently starting.")
        }
        (ServerState::Serving, LauncherMode::Manual) => {
            format!("Model '{model}' is currently being served.")
        }
        (ServerState::Stopping, LauncherMode::Manual) => {
            format!("Model '{model}' is currently stopping.")
        }
        (ServerState::Starting, LauncherMode::Router) => format!(
            "Router mode is starting; it can keep {} loaded.",
            model_count(app.llama.router.models_max)
        ),
        (ServerState::Serving, LauncherMode::Router) => format!(
            "Router mode is active; it can keep {} loaded.",
            model_count(app.llama.router.models_max)
        ),
        (ServerState::Stopping, LauncherMode::Router) => format!(
            "Router mode is stopping; {} may still be loaded.",
            model_count(app.llama.router.models_max)
        ),
        (ServerState::Error(_), LauncherMode::Manual) => {
            format!("No model is currently served; the last launch of '{model}' failed.")
        }
        (ServerState::Error(_), LauncherMode::Router) => {
            "No models are currently served; router mode failed.".to_string()
        }
        _ => "No model is currently being served.".to_string(),
    }
}

fn model_count(count: u32) -> String {
    match count {
        1 => "up to 1 model".to_string(),
        n => format!("up to {n} models"),
    }
}

/// The delete prompt: what goes, how much of it, and what goes with it.
///
/// Its own modal rather than another `Confirm` variant, because it is the
/// only prompt in the program whose `y` destroys something. It states the
/// size, names any other quantisation sharing the directory — a prompt
/// that took a second model silently would not have asked the question the
/// user answered — and says outright that it cannot be undone.
pub fn render_delete(frame: &mut Frame, app: &App, area: Rect) {
    let Some(pending) = app.llama.pending_delete.as_ref() else {
        return;
    };

    let size = pending
        .bytes
        .map(crate::services::llama::hub::human_bytes)
        .unwrap_or_else(|| "an unknown amount".into());

    let mut lines = vec![
        Line::from(""),
        Line::styled(
            format!("  Delete {} from the model cache?", pending.repo),
            Theme::status_error(),
        ),
        Line::styled(
            format!("  This frees {size} and cannot be undone."),
            Theme::normal(),
        ),
    ];

    for also in &pending.also_removes {
        lines.push(Line::styled(
            format!("  It also removes {also}"),
            Theme::status_starting(),
        ));
    }

    lines.push(Line::styled(
        "  Re-downloading it later is the only way back.".to_string(),
        Theme::logs(),
    ));
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "  Delete?   [y] yes   [any other key] cancel".to_string(),
        Theme::normal(),
    ));

    let popup = centered(area, 72, (lines.len() + 2) as u16);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).style(Theme::normal()).block(
            Block::default()
                .title(" Delete cached model ")
                .borders(Borders::ALL)
                .border_style(Theme::status_error()),
        ),
        popup,
    );
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let Some(reason) = app.llama.confirm.as_ref() else {
        return;
    };
    let model = app.llama.pending_launch.clone().unwrap_or_default();
    let popup = centered(area, 68, 8);

    let mut lines = vec![Line::from("")];
    for text in explain(reason) {
        lines.push(Line::styled(format!("  {text}"), Theme::status_error()));
    }
    lines.push(Line::styled(format!("  {}", advice(reason)), Theme::logs()));
    lines.push(Line::from(""));
    lines.push(Line::styled(
        format!("  Launch '{model}' anyway?   [y] yes   [any other key] cancel"),
        Theme::normal(),
    ));

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).style(Theme::normal()).block(
            Block::default()
                .title(title(reason))
                .borders(Borders::ALL)
                .border_style(Theme::status_error()),
        ),
        popup,
    );
}

fn title(reason: &Confirm) -> &'static str {
    match reason {
        Confirm::PortInUse { .. } => " Port in use ",
        Confirm::TooLarge { .. } => " Not enough memory ",
        Confirm::NotDownloaded { .. } => " Not downloaded ",
    }
}

/// The problem, in the user's terms.
fn explain(reason: &Confirm) -> Vec<String> {
    match reason {
        Confirm::PortInUse { port, .. } => vec![format!("Port {port} is already in use.")],
        Confirm::TooLarge { estimate, budget } => vec![format!(
            "This preset is estimated at {estimate:.1} GiB, over the {budget:.1} GiB budget."
        )],
        Confirm::NotDownloaded { repo } => vec![
            "The weights for this preset are not on this machine.".to_string(),
            format!("It would be fetched from {repo}."),
        ],
    }
}

/// What follows from it, so the modal is not just an obstacle.
fn advice(reason: &Confirm) -> String {
    match reason {
        Confirm::PortInUse { .. } => {
            "herd did not start that process and will not stop it.".to_string()
        }
        Confirm::TooLarge { .. } => {
            "Loading it may swap and stall the whole machine, not just the server.".to_string()
        }
        Confirm::NotDownloaded { .. } => {
            "Several gigabytes over your connection, then it launches.".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The prompt as one blob of text, spans flattened.
    fn quit_text(app: &App) -> String {
        quit_lines(app)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.clone())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn app_with_server(state: ServerState, mode: LauncherMode, model: Option<&str>) -> App {
        let mut app = App::with_config_path(PathBuf::from("missing-models.ini"));
        app.llama.server.state = state;
        app.llama.server.mode = mode;
        app.llama.server.model = model.map(str::to_string);
        app
    }

    #[test]
    fn quit_summary_distinguishes_idle_manual_and_router_modes() {
        let idle = app_with_server(ServerState::Off, LauncherMode::Idle, None);
        assert_eq!(
            serving_summary(&idle),
            "No model is currently being served."
        );

        let manual = app_with_server(
            ServerState::Serving,
            LauncherMode::Manual,
            Some("gemma4-12b"),
        );
        assert_eq!(
            serving_summary(&manual),
            "Model 'gemma4-12b' is currently being served."
        );

        let mut router = app_with_server(ServerState::Serving, LauncherMode::Router, None);
        router.llama.router.models_max = 3;
        assert_eq!(
            serving_summary(&router),
            "Router mode is active; it can keep up to 3 models loaded."
        );
    }

    /// The list is honest in both directions: a download is named as
    /// work at stake, and named as the one item that comes back.
    #[test]
    fn a_download_is_listed_and_said_to_be_resumable() {
        let mut app = app_with_server(ServerState::Off, LauncherMode::Idle, None);
        app.llama.download = Some(crate::app::Download {
            model: "gemma4-12b".into(),
            done: 2_000_000_000,
            total: 6_400_000_000,
        });

        let text = quit_text(&app);
        assert!(text.contains("gemma4-12b"), "{text}");
        assert!(text.contains("resumes"), "{text}");
        assert!(
            text.contains("No supervised server"),
            "nothing is serving, and the prompt should say so: {text}"
        );
    }

    /// Nothing to resume, nothing said about resuming.
    #[test]
    fn a_prompt_with_no_download_says_nothing_about_resuming() {
        let app = app_with_server(
            ServerState::Serving,
            LauncherMode::Manual,
            Some("gemma4-12b"),
        );

        let text = quit_text(&app);
        assert!(!text.contains("resumes"), "{text}");
    }

    #[test]
    fn router_model_count_uses_singular_and_plural_grammar() {
        assert_eq!(model_count(1), "up to 1 model");
        assert_eq!(model_count(2), "up to 2 models");
    }

    /// Each reason must say something of its own: a modal that reads the
    /// same whatever went wrong tells the user nothing.
    #[test]
    fn each_reason_explains_itself_differently() {
        let port = Confirm::PortInUse {
            port: 1234,
            retry: "launch! gemma4-12b".into(),
        };
        let memory = Confirm::TooLarge {
            estimate: 18.2,
            budget: 12.0,
        };

        assert_ne!(title(&port), title(&memory));
        assert_ne!(explain(&port), explain(&memory));
        assert_ne!(advice(&port), advice(&memory));
    }

    #[test]
    fn the_memory_warning_states_both_numbers() {
        let text = explain(&Confirm::TooLarge {
            estimate: 18.2,
            budget: 12.0,
        })
        .join(" ");
        assert!(text.contains("18.2"), "no estimate: {text}");
        assert!(text.contains("12.0"), "no budget: {text}");
    }
}
