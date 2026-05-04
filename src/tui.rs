use crate::{
    app::App,
    components::{command_bar, logs, sidebar, status},
    layout,
    theme::Theme,
};
use anyhow::Result;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, widgets::Block, Terminal};
use std::io;

type Backend = CrosstermBackend<io::Stdout>;

pub struct TerminalSession {
    terminal: Terminal<Backend>,
}

impl TerminalSession {
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;

        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        Ok(Self { terminal })
    }

    pub fn draw(&mut self, app: &App) -> Result<()> {
        self.terminal.draw(|frame| {
            let areas = layout::main(frame.area());

            frame.render_widget(Block::default().style(Theme::background()), frame.area());
            frame.render_widget(sidebar::view(app), areas.sidebar);
            frame.render_widget(logs::view(app), areas.main);
            frame.render_widget(command_bar::view(app), areas.command);
            frame.render_widget(status::view(app), areas.status);
        })?;

        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}
