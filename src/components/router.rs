//! The Router screen: llama-server's own multi-model mode.
//!
//! Router mode is one long-lived process pointed at the whole `models.ini`
//! that loads and unloads presets on demand — the opposite trade to the
//! Models screen, which owns exactly one preset at a time. It was
//! reachable only by typing `:router`, with its two numbers passed as
//! flags nobody could see; this screen is where they live, next to the
//! state they produce and the argv they build.

use crate::{
    app::{App, Screen},
    components, keys,
    services::llama::LauncherMode,
    theme::Theme,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(7), Constraint::Length(8)])
        .split(area);

    settings(frame, app, chunks[0]);
    preview(frame, app, chunks[1]);
}

fn settings(frame: &mut Frame, app: &App, area: Rect) {
    let rows = app.llama.router_rows();

    let block = block(format!(
        " Router · {} ",
        app.llama.tier_name().unwrap_or("no tier")
    ))
    .title_top(
        Line::styled(
            components::position(app.llama.router_cursor, rows.len()),
            Theme::border(),
        )
        .right_aligned(),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            // state, tier, endpoint, a blank line
            Constraint::Length(4),
            Constraint::Min(2),
            Constraint::Length(1),
        ])
        .split(inner)
        .to_vec();

    frame.render_widget(Paragraph::new(status(app)), chunks[0]);

    // A `List`, like every other list in herd: the cursor has to stay
    // visible, and a `Paragraph` would let ratatui clip it away.
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let selected = index == app.llama.router_cursor;
            ListItem::new(Line::styled(
                format!(
                    "{} {:<20} {:>6}   {}",
                    if selected { '▸' } else { ' ' },
                    row.key,
                    row.value_label(),
                    row.describes
                ),
                if selected {
                    Theme::selected()
                } else {
                    Theme::normal()
                },
            ))
        })
        .collect();

    let mut state = ListState::default().with_selected(Some(app.llama.router_cursor));
    frame.render_stateful_widget(List::new(items), chunks[1], &mut state);

    frame.render_widget(
        Paragraph::new(Line::styled(
            format!(
                "  {}",
                keys::screen_hint_within(
                    Screen::Router,
                    components::hint_width(chunks[2].width, false, 0)
                )
            ),
            Theme::logs(),
        )),
        chunks[2],
    );
}

/// What the router is doing, and what it would be doing to.
///
/// The state line reports the supervised process **only when it is the
/// router**: a single preset serving on the Models screen is not this
/// screen's business, and showing SERVING here for it would say the
/// router was up when it is not.
fn status(app: &App) -> Vec<Line<'static>> {
    let server = &app.llama.server;
    let is_router = server.mode == LauncherMode::Router;

    let mut state = vec![Span::styled("  state     ", Theme::logs())];
    if is_router {
        state.push(Span::styled(
            server.state.label(),
            super::server::state_style(&server.state),
        ));
        if let Some(phase) = server.phase.label() {
            state.push(Span::styled(format!("  {phase}"), Theme::status_starting()));
        }
        if let Some(elapsed) = server.elapsed_label() {
            state.push(Span::styled(format!("  {elapsed}"), Theme::logs()));
        }
    } else if server.state.is_live() {
        state.push(Span::styled(
            format!(
                "not running — {} is serving a single preset",
                server.model.clone().unwrap_or_else(|| "something".into())
            ),
            Theme::logs(),
        ));
    } else {
        state.push(Span::styled("not running", Theme::logs()));
    }

    vec![
        Line::from(state),
        field("presets", app.llama.config_path.display().to_string()),
        field(
            "endpoint",
            match is_router {
                true => server.endpoint.clone().unwrap_or_else(|| "-".into()),
                false => "-".into(),
            },
        ),
        Line::from(""),
    ]
}

fn field(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<12}"), Theme::logs()),
        Span::styled(value, Theme::normal()),
    ])
}

/// The same live argv the Models screen shows, for the same reason: the
/// numbers above are only meaningful as the flags they become.
fn preview(frame: &mut Frame, app: &App, area: Rect) {
    components::argv_preview(
        frame,
        area,
        Screen::Router,
        app.llama.router_argv_preview(),
        app.llama.preview_scroll,
    );
}

fn block(title: String) -> Block<'static> {
    Block::default()
        .title(Span::styled(title, Theme::normal()))
        .borders(Borders::ALL)
        .border_style(Theme::border())
}
