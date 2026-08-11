pub mod command_bar;
pub mod confirm;
pub mod help;
pub mod logs;
pub mod models;
pub mod picker;
pub mod server;
pub mod settings;
pub mod sidebar;
pub mod stats;
pub mod status;
pub mod test;

use ratatui::layout::Rect;

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

#[cfg(test)]
mod tests {
    use super::*;

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
