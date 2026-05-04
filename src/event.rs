use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum UiEvent {
    Key(KeyEvent),
    Tick,
    CommandFinished { command: String, output: String },
    Quit,
}

pub struct EventStream {
    running: Arc<AtomicBool>,
}

impl EventStream {
    pub fn start(tx: mpsc::UnboundedSender<UiEvent>) -> Self {
        let running = Arc::new(AtomicBool::new(true));

        Self::spawn_keyboard_reader(tx.clone(), running.clone());
        Self::spawn_tick_reader(tx, running.clone());

        Self { running }
    }

    fn spawn_keyboard_reader(tx: mpsc::UnboundedSender<UiEvent>, running: Arc<AtomicBool>) {
        tokio::task::spawn_blocking(move || {
            while running.load(Ordering::Relaxed) {
                match event::poll(Duration::from_millis(50)) {
                    Ok(true) => match event::read() {
                        Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                            if tx.send(UiEvent::Key(key)).is_err() {
                                break;
                            }
                        }
                        Ok(_) => {}
                        Err(_) => {
                            let _ = tx.send(UiEvent::Quit);
                            break;
                        }
                    },
                    Ok(false) => {}
                    Err(_) => {
                        let _ = tx.send(UiEvent::Quit);
                        break;
                    }
                }
            }
        });
    }

    fn spawn_tick_reader(tx: mpsc::UnboundedSender<UiEvent>, running: Arc<AtomicBool>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(250));

            while running.load(Ordering::Relaxed) {
                interval.tick().await;

                if tx.send(UiEvent::Tick).is_err() {
                    break;
                }
            }
        });
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
