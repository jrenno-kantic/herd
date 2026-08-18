//! The `:about` overlay: which build this is, and what it is running
//! against.
//!
//! Deliberately not decorative. `--version` exists because a version number
//! alone cannot identify a build, and this is that answer on screen — plus
//! the handful of facts that decide how herd behaves on *this* machine and
//! that a bug report is useless without: which `models.ini` is loaded,
//! which tier that is, how much RAM was detected, and what the budget
//! works out to.
//!
//! Everything here is already on screen somewhere, and that is the point:
//! the sidebar has the version, the Models title has the path, the Stats
//! screen has the budget. Answering "what am I running?" should not be a
//! tour of four screens.

use crate::{app::App, components::centered, theme::Theme, version};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// The description from `Cargo.toml`, so the two cannot drift.
const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");

/// Box width, and the two indents inside it. Named rather than scattered,
/// because the value column is measured off them: a path clipped at the
/// border reads as a path that ends there, which is the same dishonesty
/// `Columns::for_width` and `keys::screen_hint_within` exist to prevent.
const WIDTH: usize = 66;
const INDENT: usize = 2;
const LABEL: usize = 8;

/// What a value has to fit in: the box, less its borders, the indent, the
/// label column and a space before the right border.
const VALUE: usize = WIDTH - 2 - INDENT - LABEL - 1;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let lines = lines(app);
    let popup = centered(
        area,
        WIDTH as u16,
        (lines.len() as u16).saturating_add(2).min(area.height),
    );

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).style(Theme::normal()).block(
            Block::default()
                .title(" About ")
                .borders(Borders::ALL)
                .border_style(Theme::border()),
        ),
        popup,
    );
}

fn lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::styled(format!("  herd {}", version::NUMBER), Theme::status_ready()),
        Line::styled(format!("  {DESCRIPTION}"), Theme::logs()),
        Line::from(""),
        field(
            "build",
            format!("{} · {}", version::COMMIT, version::COMMIT_DATE),
        ),
    ];

    // Only when it is true, and in the colour that means "worth noticing":
    // a build from a dirty tree is one nobody else can reproduce, which is
    // exactly the sort of thing that wastes an afternoon in a bug report.
    if version::COMMIT.ends_with("-dirty") {
        lines.push(Line::styled(
            "                uncommitted changes — not reproducible".to_string(),
            Theme::status_starting(),
        ));
    }

    lines.push(Line::from(""));
    lines.push(field("config", app.llama.config_path.display().to_string()));
    lines.push(field(
        "tier",
        app.llama.tier_name().unwrap_or("-").to_string(),
    ));
    lines.push(field(
        "memory",
        match app.llama.ram_gib {
            Some(gib) => format!(
                "{gib} GiB installed · {:.1} GiB for models",
                app.llama.budget().available_gib()
            ),
            // The same restraint as everywhere else: an unreadable RAM
            // figure is not a zero, and the budget derived from it would
            // be a number nobody should act on.
            None => "unknown — presets are never flagged as too large".to_string(),
        },
    ));
    lines.push(field(
        "cache",
        match crate::services::llama::hub::hub_dir() {
            Some(dir) => dir.display().to_string(),
            None => "no HuggingFace cache on this machine".to_string(),
        },
    ));
    lines.push(field("llama", app.tools.llama_server.label().to_string()));
    lines.push(field("hf", app.tools.hf.label().to_string()));

    lines.push(Line::from(""));
    lines.push(Line::styled(
        "  esc close · ? keys · :help commands".to_string(),
        Theme::logs(),
    ));
    lines
}

fn field(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{}{label:<LABEL$}", " ".repeat(INDENT)),
            Theme::logs(),
        ),
        Span::styled(elide(&value, VALUE), Theme::normal()),
    ])
}

/// Cuts an over-long value from the **left**, keeping the tail.
///
/// These values are mostly paths, and a path is identified by its end:
/// `…/data/32gb/models.ini` says which config is loaded, where
/// `/Users/jrenno/Documents/dev…` says only whose machine it is. Same
/// reasoning as the Hub screen's repo column.
fn elide(text: &str, width: usize) -> String {
    let length = text.chars().count();
    if length <= width {
        return text.to_string();
    }
    if width <= 1 {
        return "…".chars().take(width).collect();
    }

    let kept: String = text.chars().skip(length - (width - 1)).collect();
    format!("…{kept}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn text(app: &App) -> String {
        lines(app)
            .iter()
            .map(|line| line.spans.iter().map(|span| span.content.clone()).collect())
            .collect::<Vec<String>>()
            .join("\n")
    }

    /// Everything `--version` prints has to be here too: this is that
    /// answer on screen, and a bug report quoting one should match the
    /// other.
    #[test]
    fn the_dialog_carries_what_a_bug_report_needs() {
        let app = App::with_config_path(PathBuf::from("/models/32gb/models.ini"));
        let text = text(&app);

        assert!(text.contains(version::NUMBER), "{text}");
        assert!(text.contains(version::COMMIT), "{text}");
        assert!(text.contains(version::COMMIT_DATE), "{text}");
        assert!(text.contains("/models/32gb/models.ini"), "{text}");
        assert!(text.contains("llama"), "{text}");
        assert!(text.contains("hf"), "{text}");
    }

    /// The description comes from `Cargo.toml` rather than being written
    /// out again here, so the two cannot drift.
    #[test]
    fn the_description_is_the_packages_own() {
        assert!(!DESCRIPTION.is_empty());
        assert!(text(&App::with_config_path(PathBuf::from("/x.ini"))).contains(DESCRIPTION));
    }

    /// A machine whose RAM could not be read says so, rather than
    /// implying a budget of zero — the same restraint as `Fit::Unknown`.
    #[test]
    fn an_unreadable_memory_figure_is_not_reported_as_a_budget() {
        let mut app = App::with_config_path(PathBuf::from("/x.ini"));
        app.llama.ram_gib = None;

        let text = text(&app);
        assert!(text.contains("unknown"), "{text}");
        assert!(!text.contains("0.0 GiB for models"), "{text}");
    }

    /// No line may run past the box: a path clipped at the border reads as
    /// a path that ends there, and the end is the half that identifies it.
    #[test]
    fn a_long_path_is_elided_from_the_left_and_never_overflows() {
        let deep = PathBuf::from("/Users/somebody/Documents/development/herd/data/32gb/models.ini");
        let app = App::with_config_path(deep);

        for line in lines(&app) {
            let width: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
            assert!(width <= WIDTH - 2, "a {width}-column line: {line:?}");
        }

        let text = text(&app);
        assert!(text.contains("32gb/models.ini"), "the tail was cut: {text}");
        assert!(text.contains('…'), "the cut was not marked: {text}");
    }
}
