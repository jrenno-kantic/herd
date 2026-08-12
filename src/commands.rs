//! The command bar's vocabulary, as data.
//!
//! The same argument as `keys.rs`, one layer down. The commands were
//! written down in two places that had no reason to agree: the match arms
//! that dispatch them (split across `App::submit_command`, `Executor::
//! run_command` and `services::scripts::run_script`) and a hand-written
//! `COMMANDS` list in `scripts.rs` — and they had already drifted. That
//! list never learned `reload`, never learned `cache`, and told the reader
//! that the Test screen was "key 3", which stopped being true when the
//! Router screen was inserted.
//!
//! This table is what `:help` renders and what the conformance tests drive,
//! so a command that is listed here must be handled by something, and a
//! command nobody can find is a command that does not exist.

/// Who actually runs a command. Read by the two conformance tests, which
/// between them drive every entry: `App` commands are answered
/// synchronously in `App::submit_command`, and everything else reaches the
/// `Executor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handler {
    /// Handled in `App::submit_command`: local, synchronous, no process.
    App,
    /// Handled in `Executor::run_command`, because it needs the shared
    /// `Supervisor` or an async call.
    Launcher,
    /// Falls through to the generic `services::scripts::run_script`.
    Script,
}

/// What the overlay groups commands under. Ordered as the sections are
/// shown, most load-bearing first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Server,
    Config,
    Other,
}

impl Group {
    pub const ALL: [Group; 3] = [Group::Server, Group::Config, Group::Other];

    pub fn label(self) -> &'static str {
        match self {
            Group::Server => "llama-server",
            Group::Config => "models.ini",
            Group::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Command {
    /// The first token, which is what dispatch matches on.
    pub name: &'static str,
    /// How it is written with its arguments, for the listing.
    pub usage: &'static str,
    pub summary: &'static str,
    pub group: Group,
    /// Read by the two conformance tests, which split the table between
    /// them; nothing in the UI shows it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub handler: Handler,
    /// Not listed: `launch!` is emitted by the confirmation prompt and
    /// never typed, so putting it in front of the user would only invite
    /// skipping a check that exists for a reason.
    pub hidden: bool,
    /// A form of the command the conformance test can safely drive against
    /// a config path that does not exist. Read by the tests only, hence
    /// the allow — nothing in the UI shows it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub probe: &'static str,
}

const fn command(
    name: &'static str,
    usage: &'static str,
    summary: &'static str,
    group: Group,
    handler: Handler,
    probe: &'static str,
) -> Command {
    Command {
        name,
        usage,
        summary,
        group,
        handler,
        hidden: false,
        probe,
    }
}

pub const ALL: &[Command] = &[
    command(
        "launch",
        "launch <model> [-- args]",
        "launch a preset, hot-swapping whatever is running",
        Group::Server,
        Handler::Launcher,
        "launch",
    ),
    Command {
        hidden: true,
        ..command(
            "launch!",
            "launch! <model>",
            "the same, skipping the port-in-use check — emitted by the prompt, not typed",
            Group::Server,
            Handler::Launcher,
            "launch!",
        )
    },
    command(
        "router",
        "router [--max N] [--idle S]",
        "start llama-server's own multi-model router",
        Group::Server,
        Handler::Launcher,
        "router",
    ),
    command(
        "stop",
        "stop",
        "stop the supervised process, even while something else is busy",
        Group::Server,
        Handler::Launcher,
        "stop",
    ),
    command(
        "status",
        "status",
        "is the server reachable, and what has it loaded",
        Group::Server,
        Handler::Launcher,
        "status",
    ),
    command(
        "ping",
        "ping <model>",
        "send one chat completion and print the reply",
        Group::Server,
        Handler::Launcher,
        "ping",
    ),
    command(
        "cache",
        "cache",
        "ask llama.cpp again what it has locally",
        Group::Config,
        Handler::Launcher,
        "cache",
    ),
    command(
        "models",
        "models",
        "re-read models.ini and report what it holds",
        Group::Config,
        Handler::App,
        "models",
    ),
    command(
        "reload",
        "reload",
        "the same thing, under the name that comes to mind first",
        Group::Config,
        Handler::App,
        "reload",
    ),
    command(
        "help",
        "help",
        "this list",
        Group::Other,
        Handler::App,
        "help",
    ),
    command(
        "sh",
        "sh <command>",
        "run a shell command and print its output",
        Group::Other,
        Handler::Script,
        "sh true",
    ),
];

/// Everything worth showing a user, in table order.
pub fn visible() -> impl Iterator<Item = &'static Command> {
    ALL.iter().filter(|command| !command.hidden)
}

/// The visible commands of one group.
pub fn in_group(group: Group) -> impl Iterator<Item = &'static Command> {
    visible().filter(move |command| command.group == group)
}

/// The command a typed line invokes, matched on its first token. What the
/// command bar names while a line is being typed.
pub fn find(line: &str) -> Option<&'static Command> {
    let name = line.split_whitespace().next()?;
    ALL.iter().find(|command| command.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_is_findable_by_its_first_token() {
        for command in ALL {
            assert_eq!(
                find(command.probe).map(|found| found.name),
                Some(command.name),
                "{} does not resolve from its own probe",
                command.name
            );
        }
        assert!(find("nonsense").is_none());
        assert!(find("").is_none());
    }

    /// The usage line has to start with the name it is filed under, or the
    /// listing sends the reader to type something that does not dispatch.
    #[test]
    fn a_usage_line_begins_with_its_own_command() {
        for command in ALL {
            assert!(
                command.usage.starts_with(command.name),
                "{} is documented as {:?}",
                command.name,
                command.usage
            );
        }
    }

    #[test]
    fn a_name_is_never_listed_twice() {
        let mut seen: Vec<&str> = Vec::new();
        for command in ALL {
            assert!(
                !seen.contains(&command.name),
                "{} listed twice",
                command.name
            );
            seen.push(command.name);
        }
    }

    /// Every group has to have something in it, or the overlay draws an
    /// empty heading.
    #[test]
    fn every_group_lists_something() {
        for group in Group::ALL {
            assert!(
                in_group(group).count() > 0,
                "{:?} has no commands",
                group.label()
            );
        }
        assert_eq!(
            visible().count() + 1,
            ALL.len(),
            "exactly one command (launch!) is hidden"
        );
    }
}
