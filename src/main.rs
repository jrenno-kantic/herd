mod app;
mod components;
mod engine;
mod event;
mod layout;
mod services;
mod theme;
mod tui;

use anyhow::Result;
use app::{Action, App};
use engine::Executor;
use event::EventStream;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    let mut app = App::new();
    let mut terminal = tui::TerminalSession::enter()?;
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    let _events = EventStream::start(event_tx.clone());
    let executor = Executor::new(event_tx);

    terminal.draw(&app)?;

    while let Some(event) = event_rx.recv().await {
        let action = app.update(event);

        match action {
            Action::None => {}
            Action::Quit => break,
            Action::RunCommand(command) => executor.run_command(command),
        }

        terminal.draw(&app)?;
    }

    Ok(())
}
