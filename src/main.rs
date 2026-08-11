mod app;
mod components;
mod engine;
mod event;
mod keys;
mod layout;
mod services;
mod theme;
mod tui;

use anyhow::Result;
use app::{Action, App};
use engine::Executor;
use event::EventStream;
use services::llama;
use services::llama::session::Session;
use std::path::PathBuf;
use tokio::sync::mpsc;

const USAGE: &str = "\
herd — terminal control plane for llama-server

Usage: herd [--config <path>]

Options:
  -c, --config <path>  Path to a llama-server models.ini preset file.
                       Overrides $HERD_LLAMA_CONFIG and the RAM tier
                       auto-detected under ~/models (16gb/, 32gb/, ...).
  -h, --help           Show this help and exit.
";

/// How many already-queued events to fold into one frame. Large enough to
/// swallow a startup log burst, small enough that the screen still updates
/// while one is arriving.
const DRAIN_LIMIT: usize = 256;

#[tokio::main]
async fn main() -> Result<()> {
    let cli_config = match parse_args(std::env::args().skip(1)) {
        Ok(Cli::Run { config }) => config,
        Ok(Cli::Help) => {
            print!("{USAGE}");
            return Ok(());
        }
        Err(error) => {
            eprintln!("herd: {error}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    // Resolved once, then shared: the Models screen and the Executor must
    // agree on which models.ini is in play.
    //
    // The remembered tier sits between the explicit sources and RAM
    // detection: an explicit choice always wins, but having switched tier
    // by hand last session should survive a restart.
    let session = Session::load();
    let explicit = cli_config.is_some() || llama::ini::config_env().is_some();
    let config_path = match session.usable_config_path() {
        Some(remembered) if !explicit => remembered,
        _ => llama::resolve_config_path(cli_config.as_deref()),
    };

    let mut app = App::restored(config_path.clone(), session.model.clone());
    let mut terminal = tui::TerminalSession::enter()?;
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    let _events = EventStream::start(event_tx.clone());
    let executor = Executor::new(event_tx, config_path);

    // Crossterm only reports a resize when one happens, so the starting
    // size has to be asked for. Without it the page keys would move by the
    // assumed 24-row default until the user happened to resize.
    if let Ok((_, height)) = crossterm::terminal::size() {
        app.update(event::UiEvent::Resize { height });
    }

    // Ask llama.cpp what it already has. Until the answer arrives every
    // preset reads as "unknown" rather than claiming anything.
    executor.refresh_cache();

    terminal.draw(&app)?;

    'events: while let Some(event) = event_rx.recv().await {
        // Handle everything already queued before drawing once.
        //
        // A loading llama-server emits its output in bursts of hundreds of
        // lines, and one full render per line is what made the UI lag
        // during exactly the phase the user is watching most closely.
        // Draining first collapses a burst into a single frame. The cap
        // keeps a firehose from starving the render altogether — the loop
        // simply comes straight back for the rest.
        let mut actions = vec![app.update(event)];
        for _ in 0..DRAIN_LIMIT {
            match event_rx.try_recv() {
                Ok(event) => actions.push(app.update(event)),
                Err(_) => break,
            }
        }

        for action in actions {
            match action {
                Action::None => {}
                Action::Quit => break 'events,
                Action::RunCommand(command) => executor.run_command(command),
                Action::ConfigPathChanged(path) => executor.set_config_path(path),
                Action::RunChat { model, prompt } => executor.run_chat(model, prompt),
                Action::Download {
                    model,
                    repo,
                    wants,
                    then_launch,
                } => executor.run_download(model, repo, wants, then_launch),
            }
        }

        terminal.draw(&app)?;
    }

    // Say what is happening before waiting for it. Shutdown is bounded,
    // but stopping a model that is paging can still take seconds, and a
    // motionless final frame is indistinguishable from a hang — which is
    // the impression this whole release set out to remove.
    app.push_log("shutting down…");
    let _ = terminal.draw(&app);

    // Best-effort but deterministic: never leave a supervised llama-server
    // process (and the GPU memory it holds), or an `hf` download, running
    // after herd exits.
    executor.shutdown().await;

    // Only the tier and the last preset — settings overrides are
    // session-only by design and never reach the disk. Written here rather
    // than inside `App::update`, which stays pure and testable.
    Session {
        config_path: Some(app.llama.config_path.clone()),
        model: app.llama.last_launched.clone(),
    }
    .save();

    Ok(())
}

/// Outcome of parsing the (deliberately tiny) CLI surface. An explicit
/// enum rather than a nested `Option`, so "help was asked for" can never be
/// confused with "no --config was given".
#[derive(Debug, PartialEq, Eq)]
enum Cli {
    Run { config: Option<PathBuf> },
    Help,
}

/// Hand-rolled rather than pulling in a parser crate for two flags.
fn parse_args(args: impl Iterator<Item = String>) -> Result<Cli, String> {
    let mut args = args;
    let mut config = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Cli::Help),
            "-c" | "--config" => match args.next() {
                Some(path) if !path.is_empty() => config = Some(PathBuf::from(path)),
                _ => return Err(format!("{arg} requires a path")),
            },
            other => match other.strip_prefix("--config=") {
                Some(path) if !path.is_empty() => config = Some(PathBuf::from(path)),
                Some(_) => return Err("--config requires a path".to_string()),
                None => return Err(format!("unknown argument '{other}'")),
            },
        }
    }

    Ok(Cli::Run { config })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, String> {
        parse_args(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn no_arguments_leaves_resolution_to_env_and_ram_tier() {
        assert_eq!(parse(&[]), Ok(Cli::Run { config: None }));
    }

    #[test]
    fn config_accepts_every_spelling() {
        let expected = Ok(Cli::Run {
            config: Some(PathBuf::from("/tmp/models.ini")),
        });
        assert_eq!(parse(&["--config", "/tmp/models.ini"]), expected);
        assert_eq!(parse(&["-c", "/tmp/models.ini"]), expected);
        assert_eq!(parse(&["--config=/tmp/models.ini"]), expected);
    }

    #[test]
    fn help_short_circuits_before_the_tui_starts() {
        assert_eq!(parse(&["--help"]), Ok(Cli::Help));
        assert_eq!(parse(&["-h"]), Ok(Cli::Help));
    }

    #[test]
    fn missing_or_unknown_arguments_are_rejected() {
        assert!(parse(&["--config"]).is_err());
        assert!(parse(&["--config="]).is_err());
        assert!(parse(&["--nope"]).is_err());
        assert!(parse(&["/tmp/models.ini"]).is_err());
    }

    /// A bare `--help` anywhere wins, even after a `--config`: the user
    /// asked to read the usage, not to start a server.
    #[test]
    fn help_wins_over_a_preceding_config() {
        assert_eq!(parse(&["--config", "/tmp/a.ini", "--help"]), Ok(Cli::Help));
    }
}
