pub mod about;
pub mod command_bar;
pub mod command_help;
pub mod confirm;
pub mod help;
pub mod hub;
pub mod logs;
pub mod models;
pub mod picker;
pub mod router;
pub mod server;
pub mod settings;
pub mod sidebar;
pub mod stats;
pub mod status;
pub mod test;

use crate::{app::Screen, keys, theme::Theme};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;

/// Every screen indents its footer by two spaces.
const INDENT: usize = 2;

/// How much room a footer really has inside a pane `pane_width` wide,
/// once the pane's borders, the standard indent and anything already on
/// the line are taken.
///
/// Passed to `keys::screen_hint_within` so hints are dropped deliberately
/// — with a marker saying so — rather than sliding off the right edge,
/// where a key that exists looks exactly like one that does not.
pub fn hint_width(pane_width: u16, bordered: bool, taken: usize) -> usize {
    let borders = if bordered { 2 } else { 0 };
    (pane_width as usize).saturating_sub(borders + INDENT + taken)
}

/// The narrowest pane an argv is worth wrapping into. Anything below it
/// is treated as this wide, since breaking a 20-character pane at its own
/// width produces a column of fragments rather than a command.
const ARGV_MIN_WIDTH: usize = 24;

/// Renders an argv the way a shell user would write it: one logical option
/// per line, so a twenty-flag command stays readable.
///
/// **Wrapped to the pane it will be drawn in**, not to a fixed column. It
/// used to break at a hard-coded 40, which on a narrow pane left a long
/// `--hf-repo unsloth/…-GGUF:UD-Q4_K_XL` running off the right edge and
/// being clipped by the terminal — the same silent loss `Columns::for_width`
/// and `screen_hint_within` exist to prevent everywhere else. A line that
/// still cannot fit — one option longer than the whole pane — is folded
/// here rather than cut by the terminal or soft-wrapped by the paragraph,
/// so the count returned is the count actually drawn.
///
/// Returns the lines rather than one string so the caller can count them,
/// which is what decides whether the pane needs a scrollbar.
pub fn wrap_argv(argv: &[String], width: usize) -> Vec<String> {
    const MARKER: usize = 2; // the trailing ` \`
    let width = width.max(ARGV_MIN_WIDTH);

    let mut lines = vec!["llama-server".to_string()];
    let mut current = String::from("  ");

    for (index, token) in argv.iter().enumerate() {
        // The whole option is measured, flag *and* value: measuring the
        // flag alone is how `--hf-repo` fitted and the 42-character repo
        // after it ran off the edge — the exact clipping this is here to
        // stop. One option is one unit; it is never split across lines.
        let value = argv
            .get(index + 1)
            .filter(|next| !next.starts_with('-'))
            .map(|next| next.chars().count() + 1)
            .unwrap_or(0);
        let would_be = current.chars().count() + token.chars().count() + value + MARKER;

        if token.starts_with('-') && current.trim().len() > 2 && would_be > width {
            lines.push(current.trim_end().to_string());
            current = String::from("  ");
        }
        current.push_str(token);
        current.push(' ');
    }

    if !current.trim().is_empty() {
        lines.push(current.trim_end().to_string());
    }

    // The continuation marks go on last, and only where they fit. Appended
    // as the line was built, a line already at the full width pushed its
    // own ` \` onto a fold of its own — a stray backslash under a repo
    // reference, which reads as part of the command.
    let last = lines.len().saturating_sub(1);
    for (index, line) in lines.iter_mut().enumerate() {
        if index < last && line.chars().count() + MARKER <= width {
            line.push_str(" \\");
        }
    }

    // A single option longer than the whole pane still has to go
    // somewhere. Folded rather than left to the terminal to cut, and
    // folded *here* rather than by the paragraph's own soft wrap, so the
    // line count the scrollbar is built from is the count actually drawn.
    lines
        .into_iter()
        .flat_map(|line| fold(line, width))
        .collect()
}

/// Breaks one over-long line into pane-width pieces.
fn fold(line: String, width: usize) -> Vec<String> {
    if line.chars().count() <= width {
        return vec![line];
    }

    let chars: Vec<char> = line.chars().collect();
    chars
        .chunks(width)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

/// The argv preview pane, shared by the Models and Router screens: the
/// same question ("what would this actually run?") asked of two different
/// argvs, so it is drawn by one function rather than two that drift.
///
/// Scrolls, because it overflows. Six rows hold a plain preset; a preset
/// with `[mono-focus]` on runs past them, and until this existed the rest
/// was cut off with nothing to say it was there. The border names the keys
/// **only when there is something below the fold** — advertising a key
/// where it would do nothing is its own small lie — and the scrollbar
/// appears under the same condition.
pub fn argv_preview(
    frame: &mut Frame,
    area: Rect,
    screen: Screen,
    argv: Result<Vec<String>, String>,
    scroll: usize,
) {
    let inner_width = (area.width as usize).saturating_sub(2);
    let height = (area.height as usize).saturating_sub(2);

    let lines = match &argv {
        Ok(argv) => wrap_argv(argv, inner_width),
        Err(error) => vec![error.clone()],
    };

    // Clamped again here: `App` counts the same lines from the terminal
    // size, but the pane is the authority on its own width, and a preview
    // scrolled past its end would show an empty box.
    let top = scroll.min(lines.len().saturating_sub(height));
    let scrollable = lines.len() > height;

    // The copy key is advertised here rather than in the footer: that line
    // is already at its width budget on a 100-column terminal, and the
    // hint belongs beside what it acts on. Read out of `keys.rs` so the
    // key named here and the key handled cannot drift apart.
    let mut hints: Vec<String> = Vec::new();
    if scrollable {
        hints.extend(keys::hint_for(screen, "J"));
    }
    hints.extend(keys::hint_for(screen, "y"));

    let mut block = Block::default()
        .title(Span::styled(" argv preview ", Theme::normal()))
        .borders(Borders::ALL)
        .border_style(Theme::border());

    if !hints.is_empty() {
        let hint = format!(" {} ", hints.join(" · "));
        // Dropped rather than overlapped when the pane is too narrow for
        // both the title and the hints — the title is what names the pane.
        if hint.chars().count() + " argv preview ".len() <= area.width as usize {
            block = block.title_top(Line::styled(hint, Theme::border()).right_aligned());
        }
    }

    frame.render_widget(
        Paragraph::new(
            lines
                .iter()
                .skip(top)
                .take(height)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .style(Theme::logs())
        .block(block),
        area,
    );

    text_scrollbar(frame, area, lines.len(), height, top);
}

/// Vertical scrollbar for a *text* pane, drawn on its right border and
/// only when there is something hidden. The list equivalent is
/// [`list_scrollbar`]; this one takes a line offset rather than a cursor.
pub fn text_scrollbar(frame: &mut Frame, area: Rect, total: usize, height: usize, top: usize) {
    if total <= height || height == 0 {
        return;
    }

    let mut state = ScrollbarState::new(total - height).position(top);

    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            // No arrow caps: they would land on the block's corners.
            .begin_symbol(None)
            .end_symbol(None),
        area.inner(ratatui::layout::Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut state,
    );
}

/// Centres a `width` x `height` box inside `area`, clamped so it always
/// fits even in a very small terminal. Shared by every modal.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);

    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// "3/8" for a list cursor, or nothing when there is no list to be
/// anywhere in.
///
/// Scrolling a list whose length and position are invisible is guesswork:
/// with more presets than rows on screen there was no way to tell a cursor
/// that had stopped at the end from one that had stopped responding.
/// Rendered one-based, because it is read by a person, not an index.
pub fn position(cursor: usize, len: usize) -> String {
    if len == 0 {
        return String::new();
    }
    format!(" {}/{len} ", cursor.min(len - 1) + 1)
}

/// Draws the position of a selection list on the right-hand border, beside
/// the rows it describes.
///
/// The `x/y` counter on the border says where the cursor is *after* the
/// fact; this says how much list lies above and below while a page key is
/// being held, which is a different question and the one asked more often.
///
/// Nothing is drawn when the whole list fits — a full-height thumb over
/// three rows would imply a list that does not exist — which is also what
/// the TODO asked for: visible only when the screen is too small to show
/// everything.
///
/// `list_area` is the rows themselves and `border_x` the column the block's
/// right border occupies, so the bar spans exactly the list and not the
/// header and footer around it.
pub fn list_scrollbar(
    frame: &mut Frame,
    list_area: Rect,
    border_x: u16,
    total: usize,
    cursor: usize,
) {
    let height = list_area.height as usize;
    if total <= height || height == 0 {
        return;
    }

    let mut state =
        ScrollbarState::new(total - height).position(viewport_top(total, height, cursor));

    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            // No arrow caps: they would land on the block's corners.
            .begin_symbol(None)
            .end_symbol(None),
        Rect {
            x: border_x,
            y: list_area.y,
            width: 1,
            height: list_area.height,
        },
        &mut state,
    );
}

/// The first row a `List` will show, given where the cursor is.
///
/// Derived rather than read back, because ratatui owns the offset and does
/// not expose it — but it is not a guess: the `ListState` is rebuilt from
/// the cursor every frame (see `models::table`), so ratatui always starts
/// from offset 0 and scrolls down only as far as it must to bring the
/// selection into view. That is this expression, and a bar drawn from
/// anything else would disagree with the rows next to it.
fn viewport_top(total: usize, height: usize, cursor: usize) -> usize {
    cursor
        .saturating_sub(height.saturating_sub(1))
        .min(total.saturating_sub(height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|t| t.to_string()).collect()
    }

    /// The clipping this replaced: `--hf-repo` fitted, the 42-character
    /// repo reference after it did not, and the terminal cut the line at
    /// the border — a preview that quietly showed a different command
    /// from the one it would run.
    #[test]
    fn no_wrapped_line_runs_past_the_pane() {
        let long = argv(&[
            "--host",
            "0.0.0.0",
            "--port",
            "1234",
            "--jinja",
            "--ctx-size",
            "32768",
            "--gpu-layers",
            "99",
            "--hf-repo",
            "unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL",
            "--alias",
            "gemma4-12b",
            "--spec-type",
            "draft-mtp",
        ]);

        for width in 0..=120 {
            for line in wrap_argv(&long, width) {
                assert!(
                    line.chars().count() <= width.max(ARGV_MIN_WIDTH),
                    "{width}: {line}"
                );
            }
        }
    }

    /// Wrapping must not lose or reorder anything: it is a rendering of
    /// the argv, and one that quietly drops a flag would be worse than no
    /// preview at all.
    #[test]
    fn every_token_survives_the_wrap() {
        let tokens = argv(&["--host", "0.0.0.0", "--port", "1234", "--jinja"]);

        for width in [24, 40, 70, 200] {
            let text = wrap_argv(&tokens, width).join("\n");
            assert!(text.starts_with("llama-server"), "{text}");
            for token in &tokens {
                assert!(text.contains(token.as_str()), "{width}: lost {token}");
            }
        }
    }

    /// A wider pane fits more per line, so it needs fewer of them. This is
    /// what makes the scroll bound depend on the terminal width.
    #[test]
    fn a_wider_pane_needs_fewer_lines() {
        let tokens = argv(&[
            "--host",
            "0.0.0.0",
            "--port",
            "1234",
            "--jinja",
            "--ctx-size",
            "32768",
            "--gpu-layers",
            "99",
            "--alias",
            "gemma4-12b",
        ]);

        assert!(wrap_argv(&tokens, 30).len() > wrap_argv(&tokens, 100).len());
    }

    /// While the selection fits on the first screenful, nothing scrolls.
    #[test]
    fn a_cursor_near_the_top_leaves_the_list_where_it_is() {
        assert_eq!(viewport_top(50, 10, 0), 0);
        assert_eq!(viewport_top(50, 10, 9), 0);
    }

    /// Past it, the list scrolls by exactly enough to keep the selection on
    /// the last row — which is what `List` draws, and so what the bar has
    /// to describe.
    #[test]
    fn a_cursor_past_the_first_screen_scrolls_by_the_difference() {
        assert_eq!(viewport_top(50, 10, 10), 1);
        assert_eq!(viewport_top(50, 10, 20), 11);
    }

    /// The end of the list is the end of the travel: the window must never
    /// run off the bottom, or the thumb would report scrollback that is not
    /// there.
    #[test]
    fn the_window_stops_at_the_end_of_the_list() {
        assert_eq!(viewport_top(50, 10, 49), 40);
        assert_eq!(viewport_top(3, 10, 2), 0, "a list that fits never scrolls");
    }

    #[test]
    fn a_position_is_one_based() {
        assert_eq!(position(0, 8), " 1/8 ");
        assert_eq!(position(7, 8), " 8/8 ");
    }

    /// An empty list has no position, and must not render "1/0".
    #[test]
    fn an_empty_list_has_no_position() {
        assert_eq!(position(0, 0), "");
    }

    /// A cursor left past the end by a reload or a filter must not print a
    /// number larger than the list.
    #[test]
    fn a_stale_cursor_is_clamped_to_the_list() {
        assert_eq!(position(99, 3), " 3/3 ");
    }

    #[test]
    fn centred_box_sits_inside_its_area() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 40,
        };
        let popup = centered(area, 64, 8);

        assert_eq!((popup.width, popup.height), (64, 8));
        assert!(popup.x + popup.width <= area.width);
        assert!(popup.y + popup.height <= area.height);
    }

    /// A terminal smaller than the modal must still produce a valid rect
    /// rather than underflowing the centring arithmetic.
    #[test]
    fn a_tiny_terminal_clamps_instead_of_underflowing() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 4,
        };
        let popup = centered(area, 64, 8);

        assert_eq!((popup.width, popup.height), (20, 4));
        assert!(popup.x + popup.width <= area.width);
    }

    #[test]
    fn a_modal_fits_a_small_terminal() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 8,
        };
        let popup = centered(area, 78, 20);

        assert_eq!((popup.width, popup.height), (30, 8));
        assert!(popup.x + popup.width <= area.width);
        assert!(popup.y + popup.height <= area.height);
    }
}
