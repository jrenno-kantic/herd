use crate::{app::App, theme::Theme};
use ratatui::widgets::{Block, Borders, List, ListItem};

const ITEMS: [&str; 3] = ["scripts", "devices", "logs"];

pub fn view(app: &App) -> List<'static> {
    render_sidebar(&ITEMS, app.selected_sidebar).block(
        Block::default()
            .title("OPS-TUI")
            .borders(Borders::ALL)
            .border_style(Theme::border()),
    )
}

fn render_sidebar<'a>(items: &'a [&'a str], selected: usize) -> List<'a> {
    let items = items.iter().enumerate().map(|(i, item)| {
        let style = if i == selected {
            Theme::selected()
        } else {
            Theme::normal()
        };

        ListItem::new(*item).style(style)
    });

    List::new(items.collect::<Vec<_>>())
}
