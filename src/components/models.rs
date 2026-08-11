//! The Models screen: the preset table for the active `models.ini`, plus a
//! live preview of the argv that launching the highlighted row would spawn.

use crate::{
    app::{App, Mode, Screen},
    components, keys,
    services::llama::{caps, hub::Availability, Fit, ServerState},
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
            // Two: what the highlighted preset *is*, then what the keys
            // do. Sharing one line meant the description pushed the key
            // hints off the right edge — and the description is the half
            // that grows, since it now spells out optimisations and
            // capabilities.
            Constraint::Length(2),
        ])
        .split(inner)
        .to_vec();

    // Chosen from the width actually available, not assumed: the fixed
    // layout this replaces was already 89 columns wide, so on a
    // 100-column terminal — 74 for this pane once the sidebar and borders
    // are taken — the last columns were being clipped off the right edge
    // with nothing to say they existed.
    let columns = Columns::for_width(chunks[0].width as usize);

    frame.render_widget(
        Paragraph::new(Line::styled(columns.header(), Theme::border())),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(format!("  {}", describes(app)), Theme::logs()),
            Line::styled(
                format!("  {}", keys::screen_hint(Screen::Models)),
                Theme::logs(),
            ),
        ]),
        chunks[2],
    );

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

        let text = columns.row(
            &marker,
            &row.name,
            &row.repo,
            &row.ctx,
            &estimate,
            &caps::tokens(&app.llama.optimisations(&row.name)),
            &caps::letters(&app.llama.capabilities(&row.name)),
            &row.spec,
            local,
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

/// Which columns fit, given the width the table actually got.
///
/// The table has outgrown any fixed layout: name, repo, context, memory,
/// optimisations, capabilities and local availability do not all fit on a
/// narrow terminal, and silently clipping the right-hand ones is the worst
/// of the options — the reader cannot tell a column that says nothing from
/// one that has been cut off.
///
/// So the least load-bearing columns are dropped instead, in a stated
/// order, and the repo column absorbs whatever is left over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Columns {
    repo: usize,
    ctx: bool,
    ram: bool,
    opt: bool,
    caps: bool,
    spec: bool,
}

/// Fixed widths. `repo` is the only elastic one.
const W_NAME: usize = 22;
const W_CTX: usize = 7;
const W_RAM: usize = 6;
const W_OPT: usize = 10;
const W_CAPS: usize = 5;
const W_SPEC: usize = 6;
const W_LOCAL: usize = 9;
const W_MARKER: usize = 2;
const REPO_MIN: usize = 12;
const REPO_MAX: usize = 30;

impl Columns {
    /// Everything on, repo at its widest.
    fn full() -> Self {
        Self {
            repo: REPO_MAX,
            ctx: true,
            ram: true,
            opt: true,
            caps: true,
            spec: true,
        }
    }

    fn width(&self) -> usize {
        // marker, name and local are never dropped: without them a row
        // cannot be identified, and "not local" is the difference between
        // a launch that takes a second and one that takes twenty minutes.
        let mut total = W_MARKER + W_NAME + 1 + self.repo + 1 + W_LOCAL;

        for (shown, width) in [
            (self.ctx, W_CTX),
            (self.ram, W_RAM),
            (self.opt, W_OPT),
            (self.caps, W_CAPS),
            (self.spec, W_SPEC),
        ] {
            if shown {
                total += width + 1;
            }
        }
        total
    }

    /// Fits the table to `width`, shrinking the repo column first and then
    /// dropping columns in increasing order of usefulness.
    fn for_width(width: usize) -> Self {
        let mut columns = Self::full();

        // Give the repo column back whatever is spare, before anything is
        // dropped — a wide terminal should show more repo, not more space.
        while columns.width() > width && columns.repo > REPO_MIN {
            columns.repo -= 1;
        }

        // Context size and the optimisation tokens are the first to go:
        // both are visible in the argv preview below, where the memory
        // estimate and the availability are not.
        //
        // SPEC outlives CAPS deliberately. `S` says only *whether*
        // speculative decoding is on; SPEC says which head does it, and
        // losing that was a real regression when CAPS first replaced it.
        for drop in [Drop::Ctx, Drop::Opt, Drop::Caps, Drop::Spec, Drop::Ram] {
            if columns.width() <= width {
                break;
            }
            match drop {
                Drop::Ctx => columns.ctx = false,
                Drop::Opt => columns.opt = false,
                Drop::Caps => columns.caps = false,
                Drop::Spec => columns.spec = false,
                Drop::Ram => columns.ram = false,
            }
        }

        while columns.width() < width && columns.repo < REPO_MAX {
            columns.repo += 1;
        }

        columns
    }

    fn header(&self) -> String {
        self.row(
            &" ".repeat(W_MARKER),
            "NAME",
            "REPO",
            "CTX",
            "RAM",
            "OPT",
            "CAPS",
            "SPEC",
            "LOCAL",
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        marker: &str,
        name: &str,
        repo: &str,
        ctx: &str,
        ram: &str,
        opt: &str,
        caps: &str,
        spec: &str,
        local: &str,
    ) -> String {
        let mut line = format!(
            "{marker}{:<W_NAME$} {:<width$}",
            truncate(name, W_NAME),
            truncate(repo, self.repo),
            width = self.repo
        );

        if self.ctx {
            line.push_str(&format!(" {ctx:>W_CTX$}"));
        }
        if self.ram {
            line.push_str(&format!(" {ram:>W_RAM$}"));
        }
        if self.opt {
            line.push_str(&format!(" {:>W_OPT$}", truncate(opt, W_OPT)));
        }
        if self.caps {
            line.push_str(&format!(" {:>W_CAPS$}", truncate(caps, W_CAPS)));
        }
        if self.spec {
            line.push_str(&format!(" {:>W_SPEC$}", truncate(spec, W_SPEC)));
        }
        line.push_str(&format!(" {local:>W_LOCAL$}"));

        line
    }
}

/// The order columns are given up in, least useful first.
enum Drop {
    Ctx,
    Opt,
    Caps,
    Spec,
    Ram,
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

/// What the highlighted preset is: the machine's memory, any overrides in
/// force, a fit warning, and the row's optimisations and capabilities in
/// words — which is also the legend for the compact OPT/CAPS columns, and
/// explains itself by sitting under the letters it decodes.
fn describes(app: &App) -> String {
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

    format!("RAM {ram}{overrides}{fit}{}", describe(app))
}

/// The selected preset's optimisations and capabilities, in words.
fn describe(app: &App) -> String {
    let Some(name) = app.llama.selected_model() else {
        return String::new();
    };

    let mut parts: Vec<String> = app
        .llama
        .optimisations(&name)
        .iter()
        .map(|opt| opt.label().to_string())
        .collect();
    parts.extend(app.llama.capabilities(&name).iter().map(|t| t.label()));

    if parts.is_empty() {
        String::new()
    } else {
        format!(" · {}", parts.join(" · "))
    }
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
mod column_tests {
    use super::*;

    /// The pane width on a 120-column terminal, once the 24-column sidebar
    /// and the block borders are taken.
    const WIDE: usize = 120 - 24 - 2;
    /// ...and on a 100-column one, which the old fixed layout overflowed.
    const NARROW: usize = 100 - 24 - 2;

    #[test]
    fn a_wide_terminal_shows_every_column() {
        let columns = Columns::for_width(WIDE);

        assert!(columns.ctx && columns.ram && columns.opt && columns.caps && columns.spec);
        assert!(
            columns.width() <= WIDE,
            "still overflows: {}",
            columns.width()
        );
    }

    /// The point of the exercise: never wider than the pane. Clipping is
    /// what made the old layout dishonest — a column cut off the right
    /// edge looks identical to one with nothing to say.
    #[test]
    fn no_width_produces_a_row_that_overflows() {
        // Below this nothing can fit: marker, name, the narrowest repo and
        // the availability column are the irreducible row.
        const FLOOR: usize = W_MARKER + W_NAME + 1 + REPO_MIN + 1 + W_LOCAL;
        assert_eq!(Columns::for_width(0).width(), FLOOR);

        for width in FLOOR..=200 {
            let columns = Columns::for_width(width);
            assert!(
                columns.width() <= width,
                "width {width} produced a {}-column row",
                columns.width()
            );
        }
    }

    /// A narrow terminal gives up columns in a stated order rather than
    /// losing whichever happened to be last.
    #[test]
    fn a_narrow_terminal_drops_the_least_useful_columns_first() {
        let columns = Columns::for_width(NARROW);

        assert!(columns.width() <= NARROW);
        // Whatever else goes, the preset must remain identifiable and its
        // availability visible — those are never dropped.
        assert!(columns.repo >= REPO_MIN);
        assert!(!columns.ctx, "ctx should go before ram");
        assert!(columns.ram, "ram is the last to go");
        // The regression this column exists to prevent: SPEC must outlive
        // CAPS, because `S` alone does not say which head is doing it.
        assert!(columns.spec, "spec went before caps");
    }

    #[test]
    fn the_header_and_a_row_line_up() {
        let columns = Columns::for_width(WIDE);
        let header = columns.header();
        let row = columns.row(
            "▸●",
            "gemma4-12b",
            "unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL",
            "32768",
            "8.1G",
            "qat ud",
            "vS",
            "mtp",
            "not local",
        );

        assert_eq!(
            header.chars().count(),
            row.chars().count(),
            "header:\n{header}\nrow:\n{row}"
        );
    }

    /// A repo reference is longer than any sane column, so it must be
    /// elided rather than allowed to push everything else off the line.
    #[test]
    fn a_long_repo_is_truncated_to_its_column() {
        let columns = Columns::for_width(WIDE);
        let row = columns.row(
            "  ",
            "n",
            "unsloth/gemma-4-26B-A4B-it-qat-GGUF:UD-Q4_K_XL",
            "1",
            "1",
            "1",
            "1",
            "1",
            "1",
        );

        assert_eq!(row.chars().count(), columns.width());
    }
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
