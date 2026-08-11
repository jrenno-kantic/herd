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
    CommandSpec {
        name: "models",
        description: "(Re)load models.ini and list the available llama-server presets.",
    },
    CommandSpec {
        name: "router [--max N] [--idle S]",
        description: "Launch llama-server in its built-in multi-model router mode.",
    },
    CommandSpec {
        name: "launch <model> [-- args]",
        description:
            "Launch llama-server with a single model preset (stops any running instance first).",
    },
    CommandSpec {
        name: "stop",
        description: "Stop the currently supervised llama-server process.",
    },
    CommandSpec {
        name: "ping <model>",
        description: "Send a minimal chat completion request (see also the Test screen, key 3).",
    },
    CommandSpec {
        name: "status",
        description: "Check whether llama-server is reachable and which models are loaded.",
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

pub fn help_text() -> String {
    COMMANDS
        .iter()
        .map(|command| format!("{:<28} {}", command.name, command.description))
        .collect::<Vec<_>>()
        .join("\n")
}
