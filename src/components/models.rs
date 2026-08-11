//! The Models screen: the preset table for the active `models.ini`, plus a
//! live preview of the argv that launching the highlighted row would spawn.

use crate::{
    app::{App, Mode, Screen},
    components, keys,
    services::llama::{hub::Availability, Fit, ServerState},
    theme::Theme,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(8)])
        .split(area);

    table(frame, app, chunks[0]);

    // The download takes over the argv preview's pane while it runs: the
    // argv is not what anyone is watching at that moment, and a bar that
    // has to share the space with eight lines of flags is not a bar.
    match &app.llama.download {
        Some(download) => download_bar(frame, download, chunks[1]),
        None => frame.render_widget(preview(app), chunks[1]),
    }
}

/// The download gauge, in bytes.
///
/// Byte counts rather than a bare percentage because "31%" of an unnamed
/// quantity says nothing about whether to go and make coffee.
fn download_bar(frame: &mut Frame, download: &crate::app::Download, area: Rect) {
    let block = block(format!(" downloading {} ", download.model));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner)
        .to_vec();

    // Nothing is known until the file list comes back from the API, and a
    // gauge sitting at 0% would read as a stalled download rather than one
    // that has not started measuring.
    let label = if download.total == 0 {
        "asking huggingface.co what this preset needs…".to_string()
    } else {
        download.label()
    };

    frame.render_widget(
        Gauge::default()
            .gauge_style(Theme::status_ready())
            .ratio(download.ratio())
            .label(label),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new("  the model launches on its own once this finishes").style(Theme::logs()),
        chunks[1],
    );
}

/// The preset table.
///
/// A `List` rather than one tall `Paragraph`: the paragraph rendered every
/// row and let ratatui clip whatever did not fit, so on a short terminal —
/// or simply a tier with more presets than rows on screen — `j` kept
/// moving a selection nobody could see. `List` owns the scroll offset and
/// keeps the selected row in view.
fn table(frame: &mut Frame, app: &App, area: Rect) {
    let title = match app.llama.tier_name() {
        Some(tier) => format!(" Models · {tier} · {} ", app.llama.config_path.display()),
        None => format!(" Models · {} ", app.llama.config_path.display()),
    };

    // Right-aligned so it sits at the far end of the border, away from the
    // path in the title, and stays put as the list scrolls.
    let block = block(title).title_top(
        Line::styled(
            components::position(app.llama.cursor, app.llama.rows().len()),
            Theme::border(),
        )
        .right_aligned(),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(error) = &app.llama.config_error {
        frame.render_widget(
            Paragraph::new(format!("config error: {error}")).style(Theme::status_error()),
            inner,
        );
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner)
        .to_vec();

    frame.render_widget(
        Paragraph::new(Line::styled(
            format!(
                "  {:<22} {:<30} {:>7} {:>6} {:>10}  {}",
                "NAME", "REPO", "CTX", "RAM", "LOCAL", "SPEC"
            ),
            Theme::border(),
        )),
        chunks[0],
    );
    frame.render_widget(Paragraph::new(footer(app)).style(Theme::logs()), chunks[2]);

    let rows = app.llama.rows();

    if rows.is_empty() {
        let message = if app.llama.filter.is_empty() {
            "no presets in this file".to_string()
        } else {
            format!("no preset matches '{}'", app.llama.filter)
        };
        frame.render_widget(Paragraph::new(message).style(Theme::logs()), chunks[1]);
        return;
    }

    let mut items: Vec<ListItem> = Vec::new();

    for (index, row) in rows.iter().enumerate() {
        let selected = index == app.llama.cursor;
        let is_active = app.llama.server.model.as_deref() == Some(row.name.as_str());
        let glyph = lifecycle_glyph(&app.llama.server.state, is_active);

        let marker = format!("{}{glyph}", if selected { '▸' } else { ' ' });

        let fit = app.llama.fit(&row.name);
        let estimate = app
            .llama
            .estimate_gib(&row.name)
            .map(|gib| format!("{gib:.1}G"))
            .unwrap_or_else(|| "-".into());

        // "not local" earns a column of its own rather than a colour: it
        // is the difference between pressing Enter and waiting a second,
        // and pressing Enter and waiting twenty minutes.
        let availability = app.llama.availability(&row.name);
        let local = availability.label().unwrap_or("");

        let text = format!(
            "{marker}{:<22} {:<30} {:>7} {:>6} {:>10}  {}",
            row.name,
            truncate(&row.repo, 30),
            row.ctx,
            estimate,
            local,
            row.spec
        );

        // A preset the machine cannot hold is called out in red even when
        // it is not the highlighted row: the point is to see it in the list.
        let style = match (selected, fit, availability) {
            (true, _, _) => Theme::selected(),
            (false, Fit::TooLarge, _) => Theme::status_error(),
            (false, _, Availability::Missing) => Theme::logs(),
            (false, Fit::Tight, _) => Theme::status_starting(),
            (false, _, _) => Theme::normal(),
        };

        items.push(ListItem::new(Line::styled(text, style)));
    }

    // Built fresh each frame, so the offset is recomputed from the cursor
    // and `render` stays a pure function of `App` — no draw-time mutation.
    let mut state = ListState::default().with_selected(Some(app.llama.cursor.min(rows.len() - 1)));
    frame.render_stateful_widget(List::new(items), chunks[1], &mut state);
}

/// Marker shown against a preset row.
///
/// Driven by the lifecycle, not merely by a name match: a model that has
/// been stopped or has died must lose its marker at once, or the row keeps
/// looking like the active server long after it is gone.
fn lifecycle_glyph(state: &ServerState, is_active: bool) -> char {
    if !is_active {
        return ' ';
    }
    match state {
        ServerState::Serving => '●',
        ServerState::Starting | ServerState::Stopping => '◐',
        ServerState::Error(_) => '✖',
        ServerState::Off => ' ',
    }
}

fn footer(app: &App) -> String {
    if app.mode == Mode::Filter {
        return format!("/{}▏", app.llama.filter);
    }

    let ram = app
        .llama
        .ram_gib
        .map(|gib| format!("{gib} GiB"))
        .unwrap_or_else(|| "unknown".into());

    let overrides = match app.llama.overrides.count() {
        0 => String::new(),
        n => format!(" · {n} override(s)"),
    };

    let fit = match app.llama.selected_model() {
        Some(name) => match app.llama.fit(&name) {
            Fit::TooLarge => "  ⚠ TOO LARGE for this machine".to_string(),
            Fit::Tight => "  ⚠ tight fit".to_string(),
            _ => String::new(),
        },
        None => String::new(),
    };

    format!(
        "RAM {ram}{overrides}{fit}   {}",
        keys::screen_hint(Screen::Models)
    )
}

fn preview(app: &App) -> Paragraph<'static> {
    let text = match app.llama.argv_preview() {
        Ok(argv) => wrap_argv(&argv),
        Err(error) => error,
    };

    Paragraph::new(text)
        .style(Theme::logs())
        .block(block(" argv preview ".to_string()))
}

/// Renders the argv the way a shell user would write it: one logical
/// option per line, so a 20-flag command stays readable.
fn wrap_argv(argv: &[String]) -> String {
    let mut lines = vec!["llama-server \\".to_string()];
    let mut current = String::from("  ");

    for token in argv {
        if token.starts_with('-') && current.trim().len() > 2 && current.len() > 40 {
            lines.push(format!("{} \\", current.trim_end()));
            current = String::from("  ");
        }
        current.push_str(token);
        current.push(' ');
    }

    if !current.trim().is_empty() {
        lines.push(current.trim_end().to_string());
    }

    lines.join("\n")
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let kept: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn block(title: String) -> Block<'static> {
    Block::default()
        .title(Span::styled(title, Theme::normal()))
        .borders(Borders::ALL)
        .border_style(Theme::border())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row that is not the active model never carries a marker, whatever
    /// the server happens to be doing.
    #[test]
    fn an_inactive_row_never_carries_a_marker() {
        for state in [
            ServerState::Off,
            ServerState::Starting,
            ServerState::Serving,
            ServerState::Stopping,
            ServerState::Error("oom".into()),
        ] {
            assert_eq!(lifecycle_glyph(&state, false), ' ', "{state:?}");
        }
    }

    /// The reported bug: after stopping, gemma4-12b kept its dot. OFF must
    /// render blank even while the row is still the last-known model.
    #[test]
    fn a_stopped_model_loses_its_marker() {
        assert_eq!(lifecycle_glyph(&ServerState::Off, true), ' ');
    }

    #[test]
    fn each_live_state_has_its_own_marker() {
        assert_eq!(lifecycle_glyph(&ServerState::Serving, true), '●');
        assert_eq!(lifecycle_glyph(&ServerState::Starting, true), '◐');
        assert_eq!(lifecycle_glyph(&ServerState::Stopping, true), '◐');
        assert_eq!(lifecycle_glyph(&ServerState::Error("x".into()), true), '✖');
    }

    #[test]
    fn truncate_keeps_short_text_intact() {
        assert_eq!(truncate("short", 10), "short");
    }

    #[test]
    fn truncate_marks_elided_text() {
        assert_eq!(truncate("abcdefghij", 5), "abcd…");
    }

    #[test]
    fn wrap_argv_starts_with_the_binary_and_keeps_every_token() {
        let argv = vec![
            "--host".to_string(),
            "0.0.0.0".to_string(),
            "--port".to_string(),
            "1234".to_string(),
        ];
        let wrapped = wrap_argv(&argv);

        assert!(wrapped.starts_with("llama-server \\"));
        for token in &argv {
            assert!(wrapped.contains(token), "{token} missing from preview");
        }
    }
}
