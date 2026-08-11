//! The `?` overlay: every key that does something right now.
//!
//! Read entirely from `keys.rs`, so a binding cannot appear here without
//! being documented, nor be documented without appearing here.

use crate::{app::App, components::centered, keys, theme::Theme};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let lines = lines(app);
    let popup = centered(
        area,
        62,
        (lines.len() as u16).saturating_add(2).min(area.height),
    );

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).style(Theme::normal()).block(
            Block::default()
                .title(format!(" Keys · {} ", app.screen.label()))
                .borders(Borders::ALL)
                .border_style(Theme::border()),
        ),
        popup,
    );
}

fn lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = vec![section(app.screen.label())];
    lines.extend(keys::for_screen(app.screen).iter().map(row));

    lines.push(Line::from(""));
    lines.push(section("anywhere"));
    lines.extend(keys::GLOBAL.iter().map(row));

    lines.push(Line::from(""));
    lines.push(Line::styled("  esc close", Theme::logs()));
    lines
}

fn section(title: &str) -> Line<'static> {
    Line::styled(format!("  {title}"), Theme::command())
}

fn row(binding: &keys::Binding) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {:<12}", binding.label), Theme::status_ready()),
        Span::styled(binding.action.to_string(), Theme::normal()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Screen;
    use std::path::PathBuf;

    fn app_on(screen: Screen) -> App {
        let mut app = App::with_config_path(PathBuf::from("/nonexistent/models.ini"));
        app.screen = screen;
        app
    }

    /// The overlay is the only place several keys are written down, so
    /// every screen has to produce a non-empty listing.
    #[test]
    fn every_screen_lists_its_own_keys_and_the_global_ones() {
        for screen in Screen::ALL {
            let app = app_on(screen);
            let text = lines(&app)
                .iter()
                .map(|line| line.spans.iter().map(|span| span.content.clone()).collect())
                .collect::<Vec<String>>()
                .join("\n");

            assert!(text.contains(screen.label()), "{screen:?} section missing");
            assert!(text.contains("quit"), "{screen:?} lost the global keys");

            for binding in keys::for_screen(screen) {
                assert!(
                    text.contains(binding.action),
                    "{screen:?} omits {:?}",
                    binding.action
                );
            }
        }
    }
}
