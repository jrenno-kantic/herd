use crate::services::llama::{api::ChatOutcome, hub::CachedModel, LlamaSnapshot};
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
    CommandFinished {
        command: String,
        output: String,
    },
    /// A single line streamed from a long-running supervised process
    /// (llama-server). Appended to the logs panel without touching the
    /// command bar's `running` state.
    Log(String),
    /// The llama-server supervisor changed state (spawned, became ready,
    /// exited, failed to start...).
    LlamaStatus(LlamaSnapshot),
    /// The configured port is already bound by a process herd did not
    /// start. The UI asks before launching rather than killing something
    /// it has no claim over.
    PortInUse {
        port: u16,
        model: String,
    },
    /// A chat probe finished (the `test_call.sh` equivalent). Carries the
    /// structured outcome rather than a formatted line, so the Test screen
    /// can show reply, latency and token stats separately.
    ChatResult(Box<Result<ChatOutcome, String>>),
    /// What `llama-server --cache-list` reports it has locally, with what
    /// each entry occupies on disk. Refreshed on startup, on reload, and
    /// after a download.
    CacheList(Vec<CachedModel>),
    /// A download is in flight. Byte counts, not percentages, so the bar
    /// and the "2.1G of 6.7G" text come from the same numbers.
    DownloadProgress {
        model: String,
        done: u64,
        total: u64,
    },
    /// It finished — `Ok` with a summary, or `Err` with the reason.
    DownloadFinished {
        model: String,
        result: Box<Result<String, String>>,
    },
    /// The terminal was resized. Carried into `App` so the page keys can
    /// move by a real screenful, and so the argv preview knows how many
    /// lines its pane hides; `App::update` stays pure, it simply remembers
    /// the last size it was told about.
    ///
    /// Every component is handed its own `Rect` at draw time and so knows
    /// its true size already. `App::update` runs nowhere near a frame and
    /// has no other way to find out — which is why the two places that
    /// need geometry (`chrome`, `preview_pane`) hand-count it from these
    /// numbers, and why both are pinned against a real render.
    Resize {
        width: u16,
        height: u16,
    },
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
                        Ok(Event::Resize(width, height)) => {
                            if tx.send(UiEvent::Resize { width, height }).is_err() {
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
