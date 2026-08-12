//! The Hub screen: what llama.cpp actually has in its cache.
//!
//! The Models screen answers "what can this tier launch"; this one answers
//! "what is on this machine", and the two are not the same list. A tier
//! names presets that were never downloaded, and the cache holds models no
//! tier names — several gigabytes each, invisible until something runs out
//! of disk. Both gaps are worth being able to see.
//!
//! The authority is the same as everywhere else here: `--cache-list`, not a
//! directory walk. llama.cpp is what will have to load the file, and it
//! refuses to list a repo whose weights are half-finished, which no
//! listing of ours would catch.

use crate::{
    app::{App, HubRow, Screen},
    components, keys,
    services::llama::hub::human_bytes,
    theme::Theme,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let rows = app.llama.hub_rows();

    let block = Block::default()
        .title(Span::styled(
            " Hub · llama.cpp model cache ",
            Theme::normal(),
        ))
        .borders(Borders::ALL)
        .border_style(Theme::border())
        .title_top(
            Line::styled(
                components::position(app.llama.hub_cursor, rows.len()),
                Theme::border(),
            )
            .right_aligned(),
        );

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            // What the cache costs, then what the keys do — the same split
            // as the Models footer, for the same reason: the summary is the
            // half that grows.
            Constraint::Length(2),
        ])
        .split(inner)
        .to_vec();

    let columns = Columns::for_width(chunks[0].width as usize);

    frame.render_widget(
        Paragraph::new(Line::styled(columns.header(), Theme::border())),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(format!("  {}", summary(app, &rows)), Theme::logs()),
            Line::styled(
                format!(
                    "  {}",
                    keys::screen_hint_within(
                        Screen::Hub,
                        components::hint_width(chunks[2].width, false, 0)
                    )
                ),
                Theme::logs(),
            ),
        ]),
        chunks[2],
    );

    // Nothing is claimed before llama.cpp has answered — an empty list and
    // an unanswered one look identical, and only one of them means the
    // machine holds no models.
    if rows.is_empty() {
        let message = match app.llama.cached.is_some() {
            true => "llama.cpp reports no models in its cache".to_string(),
            false => "asking llama-server what it has…".to_string(),
        };
        frame.render_widget(Paragraph::new(message).style(Theme::logs()), chunks[1]);
        return;
    }

    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let selected = index == app.llama.hub_cursor;
            let text = columns.row(
                if selected { "▸ " } else { "  " },
                &row.reference,
                &size(row.weights),
                &disk(row),
                row.preset.as_deref().unwrap_or("—"),
            );

            let style = match (selected, row.preset.is_some()) {
                (true, _) => Theme::selected(),
                // The one thing this screen is *for*: a model taking up
                // disk that nothing in this tier can launch.
                (false, false) => Theme::unreferenced(),
                (false, true) => Theme::normal(),
            };

            ListItem::new(Line::styled(text, style))
        })
        .collect();

    let mut state =
        ListState::default().with_selected(Some(app.llama.hub_cursor.min(rows.len() - 1)));
    frame.render_stateful_widget(List::new(items), chunks[1], &mut state);

    components::list_scrollbar(
        frame,
        chunks[1],
        area.x + area.width.saturating_sub(1),
        rows.len(),
        app.llama.hub_cursor,
    );
}

/// What the cache costs, and how much of it this tier can use.
fn summary(app: &App, rows: &[HubRow]) -> String {
    if rows.is_empty() {
        return String::new();
    }

    let unreferenced = rows.iter().filter(|row| row.preset.is_none()).count();
    let disk = human_bytes(app.llama.hub_disk_bytes());

    let tail = match unreferenced {
        0 => "every model has a preset in this tier".to_string(),
        n => format!("{n} not named by this tier (in cyan)"),
    };

    format!("{} model(s) · {disk} on disk · {tail}", rows.len())
}

fn size(bytes: Option<u64>) -> String {
    bytes.map(human_bytes).unwrap_or_else(|| "-".into())
}

/// The repo's disk usage, marked when it is shared.
///
/// `*` rather than a divided figure: two quantisations of one repo share a
/// blobs directory and the cache keeps no per-quantisation accounting, so
/// splitting the total would be inventing a number. The mark is explained
/// in the footer legend.
fn disk(row: &HubRow) -> String {
    match (row.disk, row.shares_disk) {
        (None, _) => "-".into(),
        (Some(bytes), false) => human_bytes(bytes),
        (Some(bytes), true) => format!("{}*", human_bytes(bytes)),
    }
}

/// Which columns fit, given the width the list actually got.
///
/// Same rule as the Models table: drop columns in a stated order rather
/// than letting them fall off the right edge, where a column that has been
/// cut off looks exactly like one with nothing to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Columns {
    reference: usize,
    disk: bool,
    preset: bool,
}

/// `1023.9M` is the widest this can render.
const W_SIZE: usize = 7;
/// The same, plus the shared-repo `*`.
const W_DISK: usize = 8;
const W_PRESET: usize = 20;
/// Selection caret and a space.
const W_MARKER: usize = 2;
const REF_MIN: usize = 20;
/// Wide enough for the longest reference on this machine —
/// `huihui-ai/Huihui-Qwen3.6-35B-A3B-Claude-4.7-Opus-abliterated-MTP-GGUF:Q4_K`
/// is 73 characters — so a wide terminal shows it whole rather than
/// eliding a name that is mostly what distinguishes it from its neighbour.
const REF_MAX: usize = 76;
/// The width below which the reference stops giving ground and something
/// else goes instead.
///
/// `unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL` is 39 characters, and the
/// shipped tiers are full of names that long. Shrinking past this turns
/// the column that identifies the row into a stub of a vendor prefix.
const REF_COMFORT: usize = 42;

impl Columns {
    fn width(&self) -> usize {
        // Marker, reference and size are never dropped: without the first
        // two a row cannot be identified, and a cache listing with no sizes
        // in it has no reason to exist.
        let mut total = W_MARKER + self.reference + 1 + W_SIZE;

        for (shown, width) in [(self.disk, W_DISK), (self.preset, W_PRESET)] {
            if shown {
                total += width + 1;
            }
        }
        total
    }

    /// Fits the table to `width`, **spending the last columns on the
    /// reference rather than on the numbers beside it.**
    ///
    /// The order matters and used to be wrong: shrinking the reference all
    /// the way to `REF_MIN` before dropping anything meant a 100-column
    /// terminal showed `…h/gemma-4-12B-it-qat-GGUF:Q4_K_XL` while still
    /// finding room for a disk figure. The name is what identifies the row;
    /// the disk usage is a number you look at once. So the reference gives
    /// ground only to `REF_COMFORT`, then DISK goes, then it gives the
    /// rest, and only then does the preset name go.
    fn for_width(width: usize) -> Self {
        let mut columns = Self {
            reference: REF_MAX,
            disk: true,
            preset: true,
        };

        // Each step runs only if the row is still too wide for the pane.
        let shrink_to = |columns: &mut Self, floor: usize| {
            while columns.width() > width && columns.reference > floor {
                columns.reference -= 1;
            }
        };

        shrink_to(&mut columns, REF_COMFORT);

        if columns.width() > width {
            columns.disk = false;
        }
        shrink_to(&mut columns, REF_MIN);

        // The preset name is last to go: the colour already says whether a
        // row is unreferenced, but it cannot say *which* preset uses it.
        if columns.width() > width {
            columns.preset = false;
        }
        shrink_to(&mut columns, REF_MIN);

        // Whatever is spare goes back to the reference — a wider terminal
        // should show more of the name, not more empty space.
        while columns.width() < width && columns.reference < REF_MAX {
            columns.reference += 1;
        }

        columns
    }

    fn header(&self) -> String {
        self.row(&" ".repeat(W_MARKER), "MODEL", "SIZE", "DISK", "PRESET")
    }

    fn row(&self, marker: &str, reference: &str, size: &str, disk: &str, preset: &str) -> String {
        let marker: String = marker
            .chars()
            .chain(std::iter::repeat(' '))
            .take(W_MARKER)
            .collect();

        let mut line = format!(
            "{marker}{:<width$} {:>W_SIZE$}",
            elide_start(reference, self.reference),
            elide_start(size, W_SIZE),
            width = self.reference
        );

        if self.disk {
            line.push_str(&format!(" {:>W_DISK$}", elide_start(disk, W_DISK)));
        }
        if self.preset {
            line.push_str(&format!(" {:<W_PRESET$}", elide_start(preset, W_PRESET)));
        }

        line
    }
}

/// Elides from the *left* of a repo reference, unlike the Models table.
///
/// The end is where the information is — `Qwen3-14B-GGUF:Q4_K_XL` — and the
/// start is a vendor name repeated on most rows. Cutting the tail off would
/// leave a column of `unsloth/Qwen3-14B-it-qat-G…` that cannot be told
/// apart.
fn elide_start(text: &str, width: usize) -> String {
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

    const WIDE: usize = 120 - 24 - 2;
    const NARROW: usize = 100 - 24 - 2;

    fn row(preset: Option<&str>, shares: bool) -> HubRow {
        HubRow {
            reference: "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL".into(),
            weights: Some(7_193_575_424),
            disk: Some(22_106_570_752),
            preset: preset.map(str::to_string),
            shares_disk: shares,
        }
    }

    /// The failure the Models table taught: a row wider than its pane is
    /// clipped without saying so.
    #[test]
    fn no_width_produces_a_row_that_overflows() {
        for width in 0..=200 {
            let columns = Columns::for_width(width);
            let line = columns.row(
                "▸ ",
                "vendor/some-model-GGUF:Q4_K_XL",
                "6.7G",
                "20.6G*",
                "p",
            );

            assert_eq!(
                line.chars().count(),
                columns.width(),
                "the row is not the width it claims at {width}"
            );
            if width >= columns.width() {
                continue;
            }
            assert!(
                columns.width() <= REF_MIN + W_MARKER + 1 + W_SIZE,
                "width {width} produced a {}-column row",
                columns.width()
            );
        }
    }

    #[test]
    fn a_wide_pane_keeps_every_column() {
        let columns = Columns::for_width(WIDE);

        assert!(columns.disk && columns.preset);
        assert!(columns.width() <= WIDE);
    }

    #[test]
    fn a_narrow_pane_gives_up_disk_before_the_preset_name() {
        let mut columns = Columns::for_width(NARROW);
        while columns.disk {
            // Walk down until something has to go, and check what went.
            columns = Columns::for_width(columns.width() - 1);
        }
        assert!(columns.preset, "the preset name went before the disk usage");
    }

    /// The reference outranks the numbers beside it.
    ///
    /// The layout this replaced shrank the reference to its minimum
    /// *before* dropping anything, so a 100-column terminal showed
    /// `…h/gemma-4-12B-it-qat-GGUF:Q4_K_XL` while still finding room for a
    /// disk figure. The name is what identifies the row.
    #[test]
    fn a_narrow_pane_spends_its_last_columns_on_the_reference() {
        let columns = Columns::for_width(NARROW);

        assert!(!columns.disk, "disk survived at the reference's expense");
        assert!(
            columns.reference >= REF_COMFORT,
            "the reference was cut to {} at {NARROW} columns",
            columns.reference
        );
        // The width that matters in practice: a full unsloth reference.
        assert!(columns.reference >= "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL".len());
    }

    /// Dropping a column must never leave more space unused than the
    /// column needed — the mistake a fixed drop order invites at the width
    /// where both would have fitted.
    #[test]
    fn no_width_wastes_room_a_dropped_column_would_have_fitted_in() {
        for width in 0..=200 {
            let columns = Columns::for_width(width);
            let slack = width.saturating_sub(columns.width());

            if !columns.disk {
                assert!(
                    slack < W_DISK + 1 || columns.reference == REF_MAX,
                    "at {width}, disk was dropped with {slack} columns to spare"
                );
            }
            if !columns.preset {
                assert!(
                    slack < W_PRESET + 1 || columns.reference == REF_MAX,
                    "at {width}, the preset name was dropped with {slack} to spare"
                );
            }
        }
    }

    #[test]
    fn the_header_and_a_row_line_up() {
        let columns = Columns::for_width(WIDE);
        assert_eq!(
            columns.header().chars().count(),
            columns
                .row("▸ ", "unsloth/x:Q4", "6.7G", "20.6G*", "gemma4-12b")
                .chars()
                .count()
        );
    }

    /// The tail of a repo reference is the part that identifies it, so it
    /// is the start that is elided.
    #[test]
    fn a_long_reference_keeps_its_quantisation() {
        let elided = elide_start("unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL", 20);

        assert_eq!(elided.chars().count(), 20);
        assert!(elided.ends_with("Q4_K_XL"), "{elided}");
        assert!(elided.starts_with('…'), "{elided}");
    }

    #[test]
    fn a_shared_repo_says_its_size_is_shared() {
        assert!(disk(&row(None, true)).ends_with('*'));
        assert!(!disk(&row(None, false)).ends_with('*'));
    }

    /// A size that was never measured is not a size of zero.
    #[test]
    fn an_unmeasured_size_renders_as_nothing_rather_than_zero() {
        assert_eq!(size(None), "-");
        assert_eq!(
            disk(&HubRow {
                disk: None,
                ..row(None, false)
            }),
            "-"
        );
    }
}
