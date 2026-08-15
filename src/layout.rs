use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub const SIDEBAR_WIDTH: u16 = 24;
pub const COMMAND_HEIGHT: u16 = 3;
pub const STATUS_HEIGHT: u16 = 1;
pub const PREVIEW_HEIGHT: u16 = 8;

#[derive(Debug, Clone, Copy)]
pub struct Areas {
    pub sidebar: Rect,
    pub main: Rect,
    pub command: Rect,
    pub status: Rect,
}

pub fn main(area: Rect) -> Areas {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(COMMAND_HEIGHT),
            Constraint::Length(STATUS_HEIGHT),
        ])
        .split(area)
        .to_vec();

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(1)])
        .split(vertical[0])
        .to_vec();

    Areas {
        sidebar: body[0],
        main: body[1],
        command: vertical[1],
        status: vertical[2],
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Split {
    pub first: Rect,
    pub second: Rect,
}

fn vertical(area: Rect, first: Constraint, second: Constraint) -> Split {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([first, second])
        .split(area);
    Split {
        first: chunks[0],
        second: chunks[1],
    }
}

pub fn with_preview(area: Rect) -> Split {
    vertical(area, Constraint::Min(3), Constraint::Length(PREVIEW_HEIGHT))
}

#[derive(Debug, Clone, Copy)]
pub struct ListAreas {
    pub header: Rect,
    pub rows: Rect,
    pub footer: Rect,
}

pub fn list(area: Rect, header: u16, footer: u16) -> ListAreas {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header),
            Constraint::Min(1),
            Constraint::Length(footer),
        ])
        .split(area);
    ListAreas {
        header: chunks[0],
        rows: chunks[1],
        footer: chunks[2],
    }
}

pub fn rows_with_footer(area: Rect, footer: u16) -> Split {
    vertical(area, Constraint::Min(1), Constraint::Length(footer))
}

pub fn server(area: Rect) -> Split {
    vertical(area, Constraint::Length(10), Constraint::Min(3))
}

/// The exact selectable rows produced by the same constraints renderers
/// consume. Keeping this here prevents input paging and drawing from
/// acquiring parallel, hand-counted versions of the layout.
pub fn page_rows(screen: crate::app::Screen, terminal: Rect) -> usize {
    let main = main(terminal).main;
    let height = match screen {
        crate::app::Screen::Models => {
            let table = with_preview(main).first.inner(ratatui::layout::Margin {
                horizontal: 1,
                vertical: 1,
            });
            list(table, 1, 2).rows.height
        }
        crate::app::Screen::Hub => {
            let inner = main.inner(ratatui::layout::Margin {
                horizontal: 1,
                vertical: 1,
            });
            list(inner, 1, 2).rows.height
        }
        crate::app::Screen::Settings => {
            let inner = main.inner(ratatui::layout::Margin {
                horizontal: 1,
                vertical: 1,
            });
            // A section header occupies two drawn rows but no selectable
            // entry. Reserve the maximum three headers when paging by
            // entry index, as the Settings renderer does.
            rows_with_footer(inner, 1).first.height.saturating_sub(6)
        }
        crate::app::Screen::Logs => {
            let inner = main.inner(ratatui::layout::Margin {
                horizontal: 1,
                vertical: 1,
            });
            rows_with_footer(inner, 1).first.height
        }
        crate::app::Screen::Router => 2,
        crate::app::Screen::Server | crate::app::Screen::Test | crate::app::Screen::Stats => 1,
    };

    // One row of overlap makes successive pages continuous.
    height.saturating_sub(1).max(3) as usize
}

/// Width and visible line count of the argv pane for the active screen.
pub fn preview_viewport(screen: crate::app::Screen, terminal: Rect) -> Option<(usize, usize)> {
    let main = main(terminal).main;
    let preview = match screen {
        crate::app::Screen::Models | crate::app::Screen::Router => with_preview(main).second,
        _ => return None,
    };
    let inner = preview.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });
    Some((inner.width as usize, inner.height as usize))
}
