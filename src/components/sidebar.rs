use crate::theme::Theme;
use ratatui::widgets::{List, ListItem};

pub fn render_sidebar<'a>(items: &'a [&'a str], selected: usize) -> List<'a> {
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
