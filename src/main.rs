mod app;
mod theme;
mod layout;
mod components;
mod services;

use crate::app::App;
use crossterm::{
    event::{self as terminal_event, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Borders},
    Terminal,
};
use std::io;
use std::time::Duration;

fn main() -> io::Result<()> {
    enable_raw_mode()?;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let mut app = App::new();

    loop {
        terminal.draw(|frame| render(frame, &app))?;

        if terminal_event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = terminal_event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Backspace => {
                        app.command_input.pop();
                    }
                    KeyCode::Enter => app.run_command(),
                    _ => app.handle_key(key),
                }
            }
        }
    }

    Ok(())
}

fn render(frame: &mut ratatui::Frame<'_>, app: &App) {
    let areas = layout::main_layout(frame.area());
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(24), Constraint::Min(1)])
        .split(areas[0]);

    let sidebar = components::sidebar::render_sidebar(&["scripts", "logs"], 0)
        .block(Block::default().title("OPS-TUI").borders(Borders::ALL));
    let logs = components::logs::render_logs(&app.logs)
        .block(Block::default().title("Logs").borders(Borders::ALL));
    let command = components::commands::render_command(&app.command_input)
        .block(Block::default().title("Command").borders(Borders::ALL));

    frame.render_widget(sidebar, body[0]);
    frame.render_widget(logs, body[1]);
    frame.render_widget(command, areas[1]);
}
