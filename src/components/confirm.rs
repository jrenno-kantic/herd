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
    theme::Theme,
};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// The exit prompt: what would be abandoned, and how to abandon it anyway.
///
/// Separate from the launch prompt because it answers a different question
/// and lists live state rather than a static reason.
pub fn render_quit(frame: &mut Frame, app: &App, area: Rect) {
    let work = app.in_flight();
    let popup = centered(area, 68, (work.len() + 7) as u16);

    let mut lines = vec![
        Line::from(""),
        Line::styled(
            "  Quitting now would abandon:".to_string(),
            Theme::status_error(),
        ),
    ];
    for item in &work {
        lines.push(Line::styled(format!("    · {item}"), Theme::normal()));
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "  The server is stopped on exit either way.".to_string(),
        Theme::logs(),
    ));
    lines.push(Line::styled(
        "  Quit anyway?   [y] yes   [any other key] stay".to_string(),
        Theme::normal(),
    ));

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).style(Theme::normal()).block(
            Block::default()
                .title(" Work in progress ")
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
        Confirm::PortInUse(_) => " Port in use ",
        Confirm::TooLarge { .. } => " Not enough memory ",
        Confirm::NotDownloaded { .. } => " Not downloaded ",
    }
}

/// The problem, in the user's terms.
fn explain(reason: &Confirm) -> Vec<String> {
    match reason {
        Confirm::PortInUse(port) => vec![format!("Port {port} is already in use.")],
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
        Confirm::PortInUse(_) => {
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

    /// Each reason must say something of its own: a modal that reads the
    /// same whatever went wrong tells the user nothing.
    #[test]
    fn each_reason_explains_itself_differently() {
        let port = Confirm::PortInUse(1234);
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
