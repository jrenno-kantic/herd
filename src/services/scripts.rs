//! What is left of the generic "run once, print the output" path herd grew
//! out of: `sh`, and nothing else.
//!
//! Three things went, all of them scaffolding from before the pivot:
//!
//! - the command *list*, a second hand-written copy of what the dispatchers
//!   accept that had drifted from them — `commands.rs` is the only place a
//!   command is written down now;
//! - `test` and `scan`, which answered fixed strings, the second without
//!   ever looking at a network;
//! - a `help` arm that could no longer fire. `:help` is intercepted in
//!   `App::submit_command` and answered as an overlay, so nothing routes it
//!   here — and an unreachable fallback is not insurance, it is a claim
//!   nobody can check.
//!
//! A 300 ms `sleep` went with them. It made a generated demo feel like it
//! was working; all it did here was tax every `:sh` by a third of a second.

pub async fn run_script(name: &str) -> String {
    match name.strip_prefix("sh ") {
        Some(command) => crate::services::system::run_shell(command).await,
        None => "Unknown command".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sh` is the one generic command left, and it matches on a prefix —
    /// a bare `sh` is a usage error rather than a shell.
    #[tokio::test]
    async fn sh_runs_the_command_it_is_given() {
        assert_eq!(run_script("sh printf hello").await, "hello");
        assert_eq!(run_script("sh").await, "Unknown command");
    }

    /// The retired scaffolding must not answer any more: a command that is
    /// no longer listed and still runs is the drift `commands.rs` exists to
    /// prevent, pointing the other way. `help` is here too — it is listed,
    /// but it is answered by the App, and reaching this path would mean the
    /// interception had broken.
    #[tokio::test]
    async fn nothing_but_sh_is_answered_here() {
        for retired in ["test", "scan", "help"] {
            assert_eq!(run_script(retired).await, "Unknown command", "{retired}");
        }
    }
}
