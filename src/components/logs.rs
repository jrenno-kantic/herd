//! The Logs screen, with real scrollback.
//!
//! The buffer is bounded (`MAX_LOGS`) but it used to be unreachable: the
//! view always rendered the tail and had no keys at all, so the line where
//! a launch failed scrolled away for good. `App::log_scroll` counts lines
//! hidden below the viewport, which is why 0 means "follow the newest
//! line" and needs no separate follow flag.

use crate::{app::App, app::Screen, components, keys, layout, theme::Theme};
use ratatui::layout::{Margin, Rect};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title("Logs")
        .borders(Borders::ALL)
        .border_style(Theme::border());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = layout::rows_with_footer(inner, 1);

    let height = chunks.first.height as usize;
    let (top, pinned) = viewport(app.logs.len(), height, app.log_scroll);

    // Deliberately not wrapped: a wrapped line counts as several rows and
    // the scroll arithmetic here counts entries, so wrapping would make
    // the position drift the longer the lines are.
    let text = app.logs.iter().cloned().collect::<Vec<_>>().join("\n");

    frame.render_widget(
        Paragraph::new(text)
            .style(Theme::logs())
            .scroll((top as u16, 0)),
        chunks.first,
    );
    frame.render_widget(
        Paragraph::new(footer(app, top, height, pinned, chunks.second.width)).style(Theme::logs()),
        chunks.second,
    );

    scrollbar(frame, app.logs.len(), height, top, area);
}

/// Draws the position of the viewport onto the right-hand border.
///
/// The footer already states it in words, but a number read after the fact
/// is not the same as seeing at a glance how much buffer is above and
/// below — which is the question being asked while holding a page key.
///
/// Nothing is drawn when the whole buffer fits: a full-height thumb on a
/// four-line log would imply a scrollback that does not exist.
fn scrollbar(frame: &mut Frame, total: usize, height: usize, top: usize, area: Rect) {
    if total <= height {
        return;
    }

    let mut state = ScrollbarState::new(total.saturating_sub(height)).position(top);

    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            // No arrow caps: they would land on the block's corners.
            .begin_symbol(None)
            .end_symbol(None),
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut state,
    );
}

/// First buffer line to show, and whether the view is scrolled back.
///
/// Anchored to the bottom: with nothing scrolled back the last line is
/// always the newest one, whatever the terminal height.
fn viewport(total: usize, height: usize, scroll: usize) -> (usize, bool) {
    let max_scroll = total.saturating_sub(height);
    let scroll = scroll.min(max_scroll);

    (max_scroll - scroll, scroll > 0)
}

fn footer(app: &App, top: usize, height: usize, pinned: bool, width: u16) -> String {
    let total = app.logs.len();
    let last = (top + height).min(total);
    let position = if pinned {
        format!("{}–{last} of {total} · scrolled back", top + 1)
    } else {
        format!("{total} lines · newest")
    };

    // The position is the load-bearing half of this line, so it is what
    // the hints have to fit around rather than the other way about.
    let hint = keys::screen_hint_within(
        Screen::Logs,
        components::hint_width(width, false, position.chars().count() + 3),
    );

    format!("  {position}   {hint}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffer_shorter_than_the_screen_starts_at_the_first_line() {
        assert_eq!(viewport(4, 20, 0), (0, false));
    }

    /// The whole point of anchoring to the bottom: following the tail
    /// shows the newest line as the last one on screen.
    #[test]
    fn following_the_tail_shows_the_end_of_the_buffer() {
        let (top, pinned) = viewport(100, 20, 0);

        assert_eq!(top, 80);
        assert!(!pinned);
        assert_eq!(top + 20, 100, "the newest line is the last one drawn");
    }

    #[test]
    fn scrolling_back_moves_the_window_towards_the_oldest_line() {
        assert_eq!(viewport(100, 20, 30), (50, true));
    }

    /// Scrolling past the oldest line must stop at it rather than running
    /// the window off the top of the buffer.
    #[test]
    fn scrollback_clamps_at_the_oldest_line() {
        assert_eq!(viewport(100, 20, 9_999), (0, true));
    }

    /// The scrollbar has to agree with the view it describes: the thumb is
    /// at the bottom when following the tail, at the top at the oldest
    /// line. `viewport` is what both are derived from, so pinning the two
    /// ends here pins the bar too.
    #[test]
    fn the_scroll_position_spans_the_whole_buffer() {
        let (total, height) = (100usize, 20usize);
        let range = total - height;

        let (following, _) = viewport(total, height, 0);
        assert_eq!(following, range, "following the tail is the far end");

        let (oldest, _) = viewport(total, height, range);
        assert_eq!(oldest, 0, "the oldest line is the near end");
    }
}
