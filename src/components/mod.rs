pub mod command_bar;
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

use ratatui::layout::Rect;
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};
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
