use crate::{event::UiEvent, services};
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct Executor {
    tx: mpsc::UnboundedSender<UiEvent>,
}

impl Executor {
    pub fn new(tx: mpsc::UnboundedSender<UiEvent>) -> Self {
        Self { tx }
    }

    pub fn run_command(&self, command: String) {
        let tx = self.tx.clone();

        tokio::spawn(async move {
            let guard = CompletionGuard::new(tx, command.clone());
            let output = services::scripts::run_script(&command).await;
            guard.complete(output);
        });
    }
}

// Ensures `CommandFinished` is always sent — even if the task panics or is
// cancelled — so the App never gets stuck with `running = true`.
struct CompletionGuard {
    tx: mpsc::UnboundedSender<UiEvent>,
    command: Option<String>,
}

impl CompletionGuard {
    fn new(tx: mpsc::UnboundedSender<UiEvent>, command: String) -> Self {
        Self {
            tx,
            command: Some(command),
        }
    }

    fn complete(mut self, output: String) {
        if let Some(command) = self.command.take() {
            let _ = self.tx.send(UiEvent::CommandFinished { command, output });
        }
    }
}

impl Drop for CompletionGuard {
    fn drop(&mut self) {
        if let Some(command) = self.command.take() {
            let _ = self.tx.send(UiEvent::CommandFinished {
                command,
                output: "task aborted".into(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn complete_sends_command_finished() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let guard = CompletionGuard::new(tx, "test".into());
        guard.complete("ok".into());

        match rx.recv().await.expect("event") {
            UiEvent::CommandFinished { command, output } => {
                assert_eq!(command, "test");
                assert_eq!(output, "ok");
            }
            other => panic!("expected CommandFinished, got {:?}", other),
        }
        assert!(rx.try_recv().is_err(), "no double send");
    }

    #[tokio::test]
    async fn drop_without_complete_sends_aborted() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        {
            let _guard = CompletionGuard::new(tx, "test".into());
        }

        match rx.recv().await.expect("event") {
            UiEvent::CommandFinished { command, output } => {
                assert_eq!(command, "test");
                assert_eq!(output, "task aborted");
            }
            other => panic!("expected CommandFinished, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn panicking_task_still_emits_completion() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(async move {
            let _guard = CompletionGuard::new(tx, "panicking".into());
            panic!("simulated panic");
        });
        let _ = handle.await;

        match rx.recv().await.expect("event") {
            UiEvent::CommandFinished { command, output } => {
                assert_eq!(command, "panicking");
                assert_eq!(output, "task aborted");
            }
            other => panic!("expected CommandFinished, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn executor_run_command_emits_completion() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let executor = Executor::new(tx);
        executor.run_command("help".into());

        match rx.recv().await.expect("event") {
            UiEvent::CommandFinished { command, .. } => {
                assert_eq!(command, "help");
            }
            other => panic!("expected CommandFinished, got {:?}", other),
        }
    }
}
