use crate::services::llama::{api::ChatOutcome, hub::CachedModel, LlamaSnapshot};
use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
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
    /// it has no claim over. `name` is what the prompt calls the thing
    /// that was refused ("gemma4-12b", "router"); `retry` is the force
    /// command a yes re-dispatches, authored by the Executor that refused
    /// it — the one place that knows the full command line, so a launch
    /// with `-- extra` args or a router with its numbers comes back
    /// verbatim rather than being rebuilt from parts.
    PortInUse {
        port: u16,
        name: String,
        retry: String,
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
    /// need geometry use the same layout functions as rendering.
    Resize {
        width: u16,
        height: u16,
    },
    Quit,
}

/// Non-blocking, bounded transport for subprocess output.
///
/// Logs are the only event class a child can produce without limit. They
/// therefore get backpressure independently from lifecycle and completion
/// events, which must never be dropped or a busy state could become stuck.
#[derive(Clone)]
pub struct LogSender {
    tx: mpsc::Sender<String>,
    dropped: Arc<AtomicUsize>,
}

pub struct LogReceiver {
    rx: mpsc::Receiver<String>,
    dropped: Arc<AtomicUsize>,
}

pub fn log_channel(capacity: usize) -> (LogSender, LogReceiver) {
    let (tx, rx) = mpsc::channel(capacity);
    let dropped = Arc::new(AtomicUsize::new(0));
    (
        LogSender {
            tx,
            dropped: dropped.clone(),
        },
        LogReceiver { rx, dropped },
    )
}

impl LogSender {
    /// Queues a line without ever slowing the child reading task. A full
    /// channel counts the loss; the receiver turns the count into one
    /// visible summary when it next gets CPU time.
    pub fn send(&self, line: String) {
        match self.tx.try_send(line) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            // The application is exiting; nobody remains to inform.
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }
}

impl LogReceiver {
    pub async fn recv(&mut self) -> Option<String> {
        self.rx.recv().await
    }

    pub fn try_recv(&mut self) -> Result<String, mpsc::error::TryRecvError> {
        self.rx.try_recv()
    }

    pub fn take_dropped(&self) -> usize {
        self.dropped.swap(0, Ordering::Relaxed)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn process_logs_are_bounded_and_report_the_overflow() {
        let (tx, mut rx) = log_channel(2);

        tx.send("one".into());
        tx.send("two".into());
        tx.send("three".into());
        tx.send("four".into());

        assert_eq!(rx.recv().await.as_deref(), Some("one"));
        assert_eq!(rx.recv().await.as_deref(), Some("two"));
        assert_eq!(rx.take_dropped(), 2);
        assert_eq!(rx.take_dropped(), 0, "the summary is emitted only once");
    }
}
