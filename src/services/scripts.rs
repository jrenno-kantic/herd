//! The generic "run once, print the output" path herd grew out of.
//!
//! Two things used to live here that no longer do. The command *list* was a
//! second, hand-written copy of what the dispatchers accept and had drifted
//! from them — `commands.rs` is the only place a command is written down
//! now. And two commands were pre-pivot scaffolding: `test`, which answered
//! the fixed string "Test executed", and `scan`, which answered
//! "No devices discovered" without ever looking at a network. Both were
//! harmless while nothing advertised them; `:help` advertises what is here,
//! and a listing that promises a scanner there is no scanner for is exactly
//! the dishonesty the rest of this codebase is built to avoid.
//!
//! `sh` stays because it does what it says: the escape hatch for the
//! occasional `:sh ls ~/models` without leaving the alternate screen.

use tokio::time::{sleep, Duration};

pub async fn run_script(name: &str) -> String {
    sleep(Duration::from_millis(300)).await;

    match name {
        // Kept as a fallback: `:help` is answered by the App as an overlay
        // and never reaches here, but a caller that does get here should
        // get the list rather than "Unknown command".
        "help" => crate::commands::help_text(),
        command if command.starts_with("sh ") => {
            crate::services::system::run_shell(&command[3..]).await
        }
        _ => "Unknown command".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sh` is the one generic command left, and the arm matches on a
    /// prefix — a bare `sh` is a usage error rather than a shell.
    #[tokio::test]
    async fn sh_runs_the_command_it_is_given() {
        assert_eq!(run_script("sh printf hello").await, "hello");
        assert_eq!(run_script("sh").await, "Unknown command");
    }

    /// The retired scaffolding must not answer any more: a command that is
    /// no longer listed and still runs is the drift `commands.rs` exists to
    /// prevent, pointing the other way.
    #[tokio::test]
    async fn the_pre_pivot_stubs_are_gone() {
        for retired in ["test", "scan"] {
            assert_eq!(run_script(retired).await, "Unknown command", "{retired}");
        }
    }
}
