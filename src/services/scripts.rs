use tokio::time::{sleep, Duration};

pub struct CommandSpec {
    pub name: &'static str,
    pub description: &'static str,
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "help",
        description: "Show available commands and descriptions.",
    },
    CommandSpec {
        name: "test",
        description: "Run a sample predefined script.",
    },
    CommandSpec {
        name: "scan",
        description: "Scan for devices through the network service.",
    },
    CommandSpec {
        name: "sh <command>",
        description: "Run a shell command asynchronously and print stdout or stderr.",
    },
];

pub async fn run_script(name: &str) -> String {
    sleep(Duration::from_millis(300)).await;

    match name {
        "help" => help_text(),
        "test" => "Test executed".into(),
        "scan" => crate::services::network::scan_devices().await,
        command if command.starts_with("sh ") => {
            crate::services::system::run_shell(&command[3..]).await
        }
        _ => "Unknown command".into(),
    }
}

fn help_text() -> String {
    COMMANDS
        .iter()
        .map(|command| format!("{:<12} {}", command.name, command.description))
        .collect::<Vec<_>>()
        .join("\n")
}
