use crate::{
    app::{App, Mode},
    components::server::state_style,
    keys,
    services::llama::ServerState,
    theme::Theme,
};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub fn view(app: &App) -> Paragraph<'static> {
    let server = &app.llama.server;
    let model = server.model.clone().unwrap_or_else(|| "-".into());

    let mut spans = vec![
        Span::styled(
            format!(" {} ", server.state.tag()),
            state_style(&server.state),
        ),
        Span::styled(format!(" {model}"), Theme::normal()),
    ];

    // The phase is the whole point of the status bar during a long load:
    // "STARTING" alone for four minutes is indistinguishable from a hang,
    // where "STARTING · loading weights · 03:41" plainly is not.
    if let Some(phase) = server.phase.label() {
        let style = if server.is_degraded() {
            Theme::status_error()
        } else {
            Theme::status_starting()
        };
        spans.push(Span::styled(format!(" · {phase}"), style));
    }
    if let Some(elapsed) = elapsed(app) {
        spans.push(Span::styled(format!(" · {elapsed}"), Theme::logs()));
    }
    if let Some(endpoint) = &server.endpoint {
        spans.push(Span::styled(format!(" · {endpoint}"), Theme::logs()));
    }
    if app.running {
        spans.push(Span::styled(" · working…", Theme::status_starting()));
    }

    spans.push(Span::styled(format!(" · {}", hint(app)), Theme::logs()));

    Paragraph::new(Line::from(spans))
}

/// Time in the current state, labelled by what that time *means*: waiting
/// while starting, uptime once serving.
fn elapsed(app: &App) -> Option<String> {
    let server = &app.llama.server;
    let elapsed = server.elapsed_label()?;

    match server.state {
        ServerState::Starting => Some(format!("waiting {elapsed}")),
        ServerState::Serving => Some(format!("up {elapsed}")),
        _ => None,
    }
}

fn hint(app: &App) -> String {
    match app.mode {
        Mode::Command => "enter run · esc cancel".to_string(),
        Mode::Filter => "type to filter · enter keep · esc clear".to_string(),
        Mode::EditSetting => "enter save · esc cancel".to_string(),
        Mode::EditPrompt => "enter use prompt · esc cancel".to_string(),
        Mode::Picker => "enter select config · esc cancel".to_string(),
        Mode::ConfirmLaunch => "y launch anyway · any other key cancel".to_string(),
        Mode::ConfirmQuit => "y quit anyway · any other key stay".to_string(),
        Mode::ConfirmDelete => "y delete · any other key cancel".to_string(),
        Mode::Help => "any key closes the help".to_string(),
        Mode::Commands => "any key closes the command list".to_string(),
        Mode::About => "any key closes this".to_string(),
        // The globals only; the screen's own keys are in its footer, and
        // both come from the same table.
        Mode::Browse => keys::global_hint(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use std::path::PathBuf;

    /// A four-minute load showing nothing but "STARTING" is what makes the
    /// launcher feel hung. The elapsed time is the cheapest possible proof
    /// that something is still happening.
    #[test]
    fn starting_and_serving_label_their_elapsed_time_differently() {
        let mut app = App::with_config_path(PathBuf::from("/nonexistent/models.ini"));

        app.update(crate::event::UiEvent::LlamaStatus(
            crate::services::llama::LlamaSnapshot::new(
                ServerState::Starting,
                crate::services::llama::LauncherMode::Manual,
                Some("gemma4-12b".into()),
            ),
        ));
        let starting = elapsed(&app).expect("starting has an elapsed time");
        assert!(starting.starts_with("waiting"), "got {starting}");

        app.llama.server.state = ServerState::Serving;
        let serving = elapsed(&app).expect("serving has an uptime");
        assert!(serving.starts_with("up"), "got {serving}");
    }

    /// Nothing is running, so there is no clock to report.
    #[test]
    fn an_idle_server_reports_no_elapsed_time() {
        let app = App::with_config_path(PathBuf::from("/nonexistent/models.ini"));
        assert_eq!(elapsed(&app), None);
    }

    #[test]
    fn each_mode_has_its_own_hint() {
        let mut app = App::with_config_path(PathBuf::from("/nonexistent/models.ini"));
        let mut seen = Vec::new();

        for mode in [
            Mode::Browse,
            Mode::Command,
            Mode::Filter,
            Mode::EditSetting,
            Mode::EditPrompt,
            Mode::Picker,
            Mode::ConfirmLaunch,
            Mode::ConfirmQuit,
            Mode::Help,
        ] {
            app.mode = mode;
            let text = hint(&app);
            assert!(!seen.contains(&text), "duplicate hint for {mode:?}");
            seen.push(text);
        }
    }
}
