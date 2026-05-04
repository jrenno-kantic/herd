use ratatui::layout::{Constraint, Direction, Layout, Rect};

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
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area)
        .to_vec();

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(24), Constraint::Min(1)])
        .split(vertical[0])
        .to_vec();

    Areas {
        sidebar: body[0],
        main: body[1],
        command: vertical[1],
        status: vertical[2],
    }
}
