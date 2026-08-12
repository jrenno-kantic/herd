//! "Copy as shell command": the argv preview, quoted so that a shell runs
//! exactly the process herd would have spawned, put on the system
//! clipboard.
//!
//! Two halves, deliberately apart:
//!
//! - [`shell_command`] is pure — argv in, one line of shell out — so the
//!   quoting can be tested against a real `sh` rather than against our own
//!   idea of what a shell reads (`the_quoting_survives_a_real_shell`).
//! - [`copy`] is the I/O, and is the only part that can fail for reasons
//!   outside the program.
//!
//! The command is emitted on **one line**, unlike the wrapped preview on
//! screen. The preview is wrapped to be read; this is meant to be pasted,
//! and a line-continuation is one more thing that can arrive mangled
//! through a chat window, an issue tracker or a paste into a shell that
//! has already got a partial line buffered.
//!
//! Clipboard access is a platform command rather than a crate, for the
//! same reason installed RAM is `sysctl`/`/proc/meminfo`: the whole job is
//! one pipe into a program that is already there.

use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

/// Bounded like everything else herd spawns. A clipboard helper that
/// never exits must not leave a task pinned for the life of the session.
const COPY_TIMEOUT: Duration = Duration::from_secs(5);

/// Characters that need no quoting in any POSIX shell. Deliberately
/// conservative: `~` expands, so it is not here, and neither is anything
/// else that a shell would look at twice.
const SAFE: &str = "_@%+=:,./-";

/// One line a shell will run: the binary, then every argv token quoted.
pub fn shell_command(binary: &str, argv: &[String]) -> String {
    let mut line = quote(binary);
    for token in argv {
        line.push(' ');
        line.push_str(&quote(token));
    }
    line
}

/// A single token, quoted the way a shell reads it back as one word.
///
/// Single quotes rather than backslashes because inside them nothing at
/// all expands — the only case to handle is a single quote itself, which
/// closes the run, is escaped, and opens the next one.
fn quote(token: &str) -> String {
    // `''` and not the empty string: an argv can legitimately carry an
    // empty argument, and dropping it silently changes the command.
    if token.is_empty() {
        return "''".to_string();
    }

    if token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || SAFE.contains(c))
    {
        return token.to_string();
    }

    format!("'{}'", token.replace('\'', r"'\''"))
}

/// Puts `text` on the system clipboard, and reports which tool took it.
///
/// Every candidate is tried in turn, so a machine with `xclip` but no
/// `wl-copy` is not a failure. When none of them works the error names
/// each one and what it said: "no clipboard tool" is not actionable,
/// "wl-copy: No such file or directory" is.
pub async fn copy(text: &str) -> Result<&'static str, String> {
    let mut refusals = Vec::new();

    for tool in TOOLS {
        match feed(tool, text).await {
            Ok(()) => return Ok(tool[0]),
            Err(error) => refusals.push(format!("{}: {error}", tool[0])),
        }
    }

    Err(refusals.join(", "))
}

/// The clipboard tools worth trying, in order.
#[cfg(target_os = "macos")]
const TOOLS: &[&[&str]] = &[&["pbcopy"]];

/// Wayland first, then the two X11 tools, since a Wayland session usually
/// carries an XWayland `xclip` that writes to a clipboard nothing reads.
#[cfg(not(target_os = "macos"))]
const TOOLS: &[&[&str]] = &[
    &["wl-copy"],
    &["xclip", "-selection", "clipboard"],
    &["xsel", "--clipboard", "--input"],
];

/// Pipes `text` into one tool. The stdin handle is dropped before the
/// wait: `pbcopy` reads until EOF, so holding it open would hang until
/// the timeout and then report a working clipboard as broken.
async fn feed(tool: &[&str], text: &str) -> Result<(), String> {
    let mut child = Command::new(tool[0])
        .args(&tool[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;

    {
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        stdin
            .write_all(text.as_bytes())
            .await
            .map_err(|error| error.to_string())?;
    }

    match timeout(COPY_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => Err(format!("exit {}", status.code().unwrap_or(-1))),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => {
            let _ = child.start_kill();
            Err(format!("no answer in {}s", COPY_TIMEOUT.as_secs()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn ordinary_flags_are_left_alone() {
        let line = shell_command(
            "llama-server",
            &argv(&[
                "--port",
                "1234",
                "--hf-repo",
                "unsloth/Qwen3-30B-GGUF:Q4_K_M",
            ]),
        );

        assert_eq!(
            line,
            "llama-server --port 1234 --hf-repo unsloth/Qwen3-30B-GGUF:Q4_K_M"
        );
    }

    #[test]
    fn a_space_or_a_quote_is_quoted() {
        assert_eq!(quote("a value"), "'a value'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote(""), "''");
        // A path a shell would expand must survive as itself.
        assert_eq!(quote("~/models/models.ini"), "'~/models/models.ini'");
        assert_eq!(quote("$HOME"), "'$HOME'");
    }

    /// The point of the exercise, and the one thing asserting on our own
    /// output cannot check: a shell has to read the line back as exactly
    /// the argv it was built from. `printf` rather than the real binary —
    /// running the copied command would launch a server.
    #[test]
    fn the_quoting_survives_a_real_shell() {
        let tokens = argv(&[
            "--chat-template-kwargs",
            r#"{"enable_thinking": false}"#,
            "--alias",
            "it's a model",
            "--prop",
            "$HOME `whoami` ~ \\ \"q\"",
            "--empty",
            "",
        ]);

        // printf's own format comes first, and goes through the same
        // quoting as everything else.
        let mut printf = argv(&["%s\n"]);
        printf.extend(tokens.clone());

        let line = shell_command("printf", &printf);
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&line)
            .output()
            .expect("sh");

        // printf reuses its format for every remaining argument, so each
        // token comes back on its own line, verbatim.
        let seen: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect();

        assert_eq!(seen, tokens, "the shell read the command back differently");
    }

    /// The half no assertion on our own output can reach: whether the
    /// clipboard actually took it. Ignored because it **replaces whatever
    /// is on the clipboard** — run it deliberately:
    ///
    /// ```text
    /// cargo test the_clipboard_really_takes_it -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore = "replaces the contents of the system clipboard"]
    async fn the_clipboard_really_takes_it() {
        let line = shell_command("llama-server", &argv(&["--alias", "a model"]));
        let tool = copy(&line).await.expect("clipboard");

        let read_back = std::process::Command::new(if cfg!(target_os = "macos") {
            "pbpaste"
        } else {
            "wl-paste"
        })
        .output()
        .expect("paste");

        assert_eq!(
            String::from_utf8_lossy(&read_back.stdout).trim_end(),
            line,
            "{tool} did not put the command on the clipboard"
        );
    }
}
