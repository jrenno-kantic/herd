//! The generic "run once, print the output" commands herd grew out of.
//!
//! The command *list* used to live here as a second, hand-written copy of
//! what the dispatchers accept, and had drifted from them — see
//! `commands.rs`, which is now the only place a command is written down.
//! What is left here is the handful this module actually runs.

use tokio::time::{sleep, Duration};

pub async fn run_script(name: &str) -> String {
    sleep(Duration::from_millis(300)).await;

    match name {
        // Kept as a fallback: `:help` is answered by the App as an overlay
        // and never reaches here, but a caller that does get here should
        // get the list rather than "Unknown command".
        "help" => crate::commands::help_text(),
        "test" => "Test executed".into(),
        "scan" => crate::services::network::scan_devices().await,
        command if command.starts_with("sh ") => {
            crate::services::system::run_shell(&command[3..]).await
        }
        _ => "Unknown command".into(),
    }
}
