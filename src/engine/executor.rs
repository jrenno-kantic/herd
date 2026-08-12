use crate::event::UiEvent;
use crate::services;
use crate::services::llama;
use crate::services::llama::{LauncherMode, LlamaSnapshot, ServerState, Supervisor};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct Executor {
    tx: mpsc::UnboundedSender<UiEvent>,
    llama: Supervisor,
    /// The `hf` process, while one is running. Held so that quitting can
    /// stop it rather than leave it writing into the cache after the UI
    /// has gone — `kill_on_drop` fires only once the task is dropped, and
    /// on a clean exit nothing drops it in time.
    download: Arc<tokio::sync::Mutex<Option<tokio::process::Child>>>,
    /// The active `models.ini`. Resolved at startup and kept in step with
    /// `App` when the user switches tier, so the Models screen and the
    /// Executor can never disagree about which file is in play.
    config_path: Arc<RwLock<PathBuf>>,
}

impl Executor {
    pub fn new(tx: mpsc::UnboundedSender<UiEvent>, config_path: PathBuf) -> Self {
        Self {
            tx,
            llama: Supervisor::new(),
            download: Arc::new(tokio::sync::Mutex::new(None)),
            config_path: Arc::new(RwLock::new(config_path)),
        }
    }

    /// Follows a tier switch made in the UI. Without this, launching after
    /// pressing `t` would resolve the preset against the *previous* tier's
    /// file and fail with "unknown model".
    pub fn set_config_path(&self, path: PathBuf) {
        if let Ok(mut slot) = self.config_path.write() {
            *slot = path;
        }
    }

    /// Snapshot of the active path. Cloned out immediately so no lock is
    /// ever held across an await point.
    fn config_path(&self) -> PathBuf {
        self.config_path
            .read()
            .map(|slot| slot.clone())
            .unwrap_or_default()
    }

    /// Dispatches a submitted `:command`. llama-server lifecycle commands
    /// (`launch`, `router`, `stop`, `ping`, `status`) are handled here
    /// because they need access to the shared `Supervisor`; everything
    /// else falls back to the generic "run once, report the output"
    /// path used by the pre-existing `services::scripts` commands.
    pub fn run_command(&self, command: String) {
        let trimmed = command.trim();

        // `launch!` is the same command with the port-in-use check already
        // answered by the user; it is emitted by the confirmation prompt,
        // never typed.
        if trimmed == "launch!" || trimmed.starts_with("launch! ") {
            let rest = trimmed
                .strip_prefix("launch!")
                .unwrap_or("")
                .trim()
                .to_string();
            return self.spawn_launch(rest, true);
        }
        if trimmed == "launch" || trimmed.starts_with("launch ") {
            let rest = trimmed
                .strip_prefix("launch")
                .unwrap_or("")
                .trim()
                .to_string();
            return self.spawn_launch(rest, false);
        }
        if trimmed == "router" || trimmed.starts_with("router ") {
            let rest = trimmed
                .strip_prefix("router")
                .unwrap_or("")
                .trim()
                .to_string();
            return self.spawn_router(rest);
        }
        if trimmed == "stop" {
            return self.spawn_stop();
        }
        if trimmed == "ping" || trimmed.starts_with("ping ") {
            let model = trimmed
                .strip_prefix("ping")
                .unwrap_or("")
                .trim()
                .to_string();
            return self.spawn_ping(model);
        }
        if trimmed == "status" {
            return self.spawn_status();
        }
        // Re-asks llama.cpp what it has. The cache changes underneath herd
        // — a download from another terminal, a repo deleted to free disk —
        // and until this existed the only way to see that was to restart.
        if trimmed == "cache" {
            return self.spawn_cache_refresh();
        }

        self.spawn_quick(command);
    }

    /// Winds everything down. Called once from `main.rs` right after the
    /// event loop exits, so quitting never leaves an orphaned
    /// `llama-server` holding GPU memory or an `hf` process still writing
    /// into the model cache.
    ///
    /// Every step is bounded (see `Supervisor::stop`), so this cannot be
    /// what makes the app hang on exit.
    pub async fn shutdown(&self) {
        self.stop_download().await;
        self.llama.stop().await;
    }

    /// Kills the downloader if one is running.
    ///
    /// A partial download is not lost work: `hf` writes to a
    /// `.incomplete` blob and resumes from it next time, which is exactly
    /// why interrupting one is safe enough to do without ceremony.
    async fn stop_download(&self) -> bool {
        let taken = self.download.lock().await.take();

        match taken {
            Some(mut child) => {
                let _ = child.start_kill();
                let _ = tokio::time::timeout(DOWNLOAD_GRACE, child.wait()).await;
                true
            }
            None => false,
        }
    }

    /// Sends a chat probe (the `test_call.sh` equivalent) and reports the
    /// structured outcome. Deliberately outside the `running` flag: a
    /// probe can take a while against a large model, and it must not lock
    /// the user out of stopping the server meanwhile.
    pub fn run_chat(&self, model: String, prompt: String) {
        let tx = self.tx.clone();
        let config_path = self.config_path();

        tokio::spawn(async move {
            let guard = ChatGuard::new(tx.clone());

            let result = match llama::load(&config_path) {
                Err(error) => Err(format!("config error: {error}")),
                Ok(config) => {
                    let base = llama::api::base_url(&config.client_host(), config.port());
                    llama::api::chat(&base, &model, &prompt).await
                }
            };

            guard.complete(result);
        });
    }

    /// Puts a line of text on the system clipboard and says so.
    ///
    /// Fire and forget, and outside the `running` flag: it is one pipe
    /// into `pbcopy`. The log line is written when the clipboard has
    /// actually taken it, never before — a "copied" that was really a
    /// missing `xclip` is worse than no key at all, because it is only
    /// discovered at the paste.
    pub fn copy(&self, label: String, text: String) {
        let tx = self.tx.clone();

        tokio::spawn(async move {
            let message = match services::clipboard::copy(&text).await {
                Ok(tool) => format!("copied the {label} launch command ({tool})"),
                Err(error) => format!("could not reach the clipboard — {error}"),
            };
            let _ = tx.send(UiEvent::Log(message));
        });
    }

    /// Asks llama.cpp what it already has, and tells the UI.
    ///
    /// Fire and forget: the answer is an improvement on "unknown", never
    /// something the app waits for, so a machine without `llama-server` on
    /// its PATH simply keeps showing nothing rather than blocking.
    pub fn refresh_cache(&self) {
        let tx = self.tx.clone();

        tokio::spawn(async move {
            match llama::hub::cache_list().await {
                Ok(cached) => {
                    let _ = tx.send(UiEvent::CacheList(cached));
                }
                Err(error) => {
                    let _ = tx.send(UiEvent::Log(format!(
                        "could not read the model cache: {error}"
                    )));
                }
            }
        });
    }

    /// Removes a cached model, then re-reads the cache.
    ///
    /// The refresh is the point: llama.cpp is the authority on what is
    /// here, so a row disappears because it asked again and got a shorter
    /// list — not because herd assumed its own deletion worked. A removal
    /// that half-succeeded therefore shows up as a row that is still
    /// listed, rather than as a screen that quietly disagrees with the
    /// disk.
    ///
    /// Outside the `running` flag, like the clipboard: it is one directory
    /// unlink, and the confirmation has already been given.
    pub fn delete_model(&self, reference: String, repo: String) {
        let tx = self.tx.clone();
        let executor = self.clone();

        tokio::spawn(async move {
            let message = match llama::hub::hub_dir() {
                None => "no HuggingFace cache directory on this machine".to_string(),
                Some(hub) => match llama::hub::delete_repo(&hub, &repo).await {
                    Ok(freed) => format!(
                        "deleted {reference} — freed {}",
                        llama::hub::human_bytes(freed)
                    ),
                    Err(error) => format!("could not delete {repo}: {error}"),
                },
            };

            let _ = tx.send(UiEvent::Log(message));
            executor.refresh_cache();
        });
    }

    /// Downloads the artifacts a preset needs, then optionally launches it.
    ///
    /// Progress is *measured*, not parsed: `hf` reports only how many
    /// files it has finished, which would leave the bar at 0% for the
    /// whole of a 6.7 GB weights file. A poller watches the bytes landing
    /// in the hub cache instead, so the bar moves smoothly and ends
    /// exactly on the total.
    pub fn run_download(
        &self,
        model: String,
        reference: String,
        wants: llama::hub::Wants,
        then_launch: bool,
    ) {
        let tx = self.tx.clone();
        let executor = self.clone();

        tokio::spawn(async move {
            let guard = CompletionGuard::new(tx.clone(), format!("download {model}"));
            let (repo, tag) = {
                let (repo, tag) = llama::hub::split_repo(&reference);
                (repo.to_string(), tag.map(str::to_string))
            };

            let files = match llama::hub::tree(&repo).await {
                Ok(files) => llama::hub::select(&files, tag.as_deref(), wants),
                Err(error) => {
                    let _ = tx.send(UiEvent::DownloadFinished {
                        model: model.clone(),
                        result: Box::new(Err(error.clone())),
                    });
                    return guard.complete(error);
                }
            };

            if files.is_empty() {
                let reason = format!("nothing in {repo} matches the quant tag in '{reference}'");
                let _ = tx.send(UiEvent::DownloadFinished {
                    model: model.clone(),
                    result: Box::new(Err(reason.clone())),
                });
                return guard.complete(reason);
            }

            let total: u64 = files.iter().map(|file| file.size).sum();
            let _ = tx.send(UiEvent::Log(format!(
                "$ {} {}",
                llama::hub::DOWNLOADER,
                llama::hub::download_args(&repo, &files).join(" ")
            )));

            let progress = spawn_progress_poller(
                tx.clone(),
                model.clone(),
                repo.clone(),
                files.clone(),
                total,
            );
            let outcome = fetch(&repo, &files, &tx, &executor.download).await;
            progress.abort();

            // A zero exit from `hf` is not proof the model is usable.
            // llama.cpp decides that, and it can still say no — a repo
            // whose `main` moved on, or a fetch that stopped short, leaves
            // the row reading "not local" while we cheerfully report
            // success. So the claim is checked before it is made.
            let verdict = match &outcome {
                Err(error) => Err(error.clone()),
                Ok(()) => {
                    let _ = tx.send(UiEvent::DownloadProgress {
                        model: model.clone(),
                        done: total,
                        total,
                    });

                    let cached = llama::hub::cache_list().await.unwrap_or_default();
                    let _ = tx.send(UiEvent::CacheList(cached.clone()));

                    match llama::hub::availability(&reference, &cached) {
                        llama::hub::Availability::Local => Ok(format!(
                            "{model} downloaded ({})",
                            llama::hub::human_bytes(total)
                        )),
                        _ => Err(format!(
                            "{} finished, but llama.cpp still does not list {repo} — \
                             the fetch stopped short, or the repo has moved on to a \
                             revision this build has not got",
                            llama::hub::DOWNLOADER
                        )),
                    }
                }
            };

            let summary = match &verdict {
                Ok(summary) => summary.clone(),
                Err(error) => error.clone(),
            };

            let _ = tx.send(UiEvent::DownloadFinished {
                model: model.clone(),
                result: Box::new(verdict.clone()),
            });

            if verdict.is_ok() && then_launch {
                executor.run_command(format!("launch {model}"));
            }

            guard.complete(summary);
        });
    }

    /// The `:cache` command: [`refresh_cache`](Self::refresh_cache) with a
    /// completion, so the busy flag it set is released and the log says
    /// what came back.
    fn spawn_cache_refresh(&self) {
        let tx = self.tx.clone();

        tokio::spawn(async move {
            let guard = CompletionGuard::new(tx.clone(), "cache".into());

            let output = match llama::hub::cache_list().await {
                Ok(cached) => {
                    let count = cached.len();
                    let _ = tx.send(UiEvent::CacheList(cached));
                    format!("{count} model(s) in the llama.cpp cache")
                }
                Err(error) => format!("could not read the model cache: {error}"),
            };

            guard.complete(output);
        });
    }

    fn spawn_quick(&self, command: String) {
        let tx = self.tx.clone();

        tokio::spawn(async move {
            let guard = CompletionGuard::new(tx, command.clone());
            let output = services::scripts::run_script(&command).await;
            guard.complete(output);
        });
    }

    fn spawn_launch(&self, rest: String, force: bool) {
        let tx = self.tx.clone();
        let supervisor = self.llama.clone();
        let config_path = self.config_path();

        tokio::spawn(async move {
            let guard = CompletionGuard::new(tx.clone(), format!("launch {rest}"));

            let (model, extra) = split_extra_args(&rest);
            if model.is_empty() {
                guard.complete("usage: launch <model> [-- extra llama-server args]".into());
                return;
            }

            let config = match llama::load(&config_path) {
                Ok(config) => config,
                Err(error) => return guard.complete(format!("config error: {error}")),
            };

            let host = config.client_host();
            let port = config.port();

            // Only worth asking when the port is held by something else.
            // If herd owns the process, `spawn` hot-swaps it cleanly.
            if !force
                && !supervisor.is_running().await
                && llama::api::port_in_use_settled(&host, port).await
            {
                let _ = tx.send(UiEvent::PortInUse {
                    port,
                    model: model.clone(),
                });
                return guard.complete(format!("port {port} busy, waiting for confirmation"));
            }

            let args = match llama::ini::build_model_args(&config, &model, &extra) {
                Ok(args) => args,
                Err(error) => return guard.complete(error),
            };

            let base_url = llama::api::base_url(&host, port);
            let output = match supervisor
                .spawn(
                    LauncherMode::Manual,
                    Some(model.clone()),
                    args,
                    base_url,
                    tx.clone(),
                    // So the health poller can tell a 16 GiB download from
                    // a server that never opened its port.
                    llama::ini::effective_repo(&config, &model),
                )
                .await
            {
                Ok(()) => format!("spawning llama-server for '{model}'"),
                Err(error) => {
                    send_error(&tx, LauncherMode::Manual, Some(model.clone()), &error);
                    error
                }
            };

            guard.complete(output);
        });
    }

    fn spawn_router(&self, rest: String) {
        let tx = self.tx.clone();
        let supervisor = self.llama.clone();
        let config_path = self.config_path();

        tokio::spawn(async move {
            let guard =
                CompletionGuard::new(tx.clone(), format!("router {rest}").trim().to_string());
            let (models_max, sleep_idle, extra) = parse_router_flags(&rest);

            let output = match llama::load(&config_path) {
                Err(error) => format!("config error: {error}"),
                Ok(config) => {
                    let args =
                        llama::ini::build_router_args(&config, models_max, sleep_idle, &extra);
                    let base_url = llama::api::base_url(&config.client_host(), config.port());
                    match supervisor
                        .spawn(LauncherMode::Router, None, args, base_url, tx.clone(), None)
                        .await
                    {
                        Ok(()) => format!(
                            "spawning llama-server router (models-max={models_max}, sleep-idle={sleep_idle}s)"
                        ),
                        Err(error) => {
                            send_error(&tx, LauncherMode::Router, None, &error);
                            error
                        }
                    }
                }
            };

            guard.complete(output);
        });
    }

    fn spawn_stop(&self) {
        let tx = self.tx.clone();
        let supervisor = self.llama.clone();

        tokio::spawn(async move {
            let guard = CompletionGuard::new(tx.clone(), "stop".into());

            guard.complete(supervisor.stop_announced(&tx).await.label().to_string());
        });
    }

    fn spawn_ping(&self, model: String) {
        let tx = self.tx.clone();
        let config_path = self.config_path();

        tokio::spawn(async move {
            let guard = CompletionGuard::new(tx.clone(), format!("ping {model}"));

            if model.is_empty() {
                guard.complete("usage: ping <model>".into());
                return;
            }

            let output = match llama::load(&config_path) {
                Err(error) => format!("config error: {error}"),
                Ok(config) => {
                    let base = llama::api::base_url(&config.client_host(), config.port());
                    match llama::api::test_chat(&base, &model).await {
                        Ok(reply) => format!("{model} -> {reply}"),
                        // The error already says what went wrong and where,
                        // so it is not wrapped again: "gemma4-12b -> error:
                        // nothing is listening on …" reads as two problems.
                        Err(error) => error,
                    }
                }
            };

            guard.complete(output);
        });
    }

    fn spawn_status(&self) {
        let tx = self.tx.clone();
        let config_path = self.config_path();

        tokio::spawn(async move {
            let guard = CompletionGuard::new(tx.clone(), "status".into());

            let output = match llama::load(&config_path) {
                Err(error) => format!("config error: {error}"),
                Ok(config) => {
                    let base = llama::api::base_url(&config.client_host(), config.port());
                    match llama::api::list_models(&base).await {
                        Ok(models) if models.is_empty() => {
                            format!("{base} reachable, no models currently loaded")
                        }
                        Ok(models) => format!("{base} reachable, loaded: {}", models.join(", ")),
                        // Likewise: `{base} unreachable: …` in front of a
                        // message that already names the base read as
                        // "unreachable: nothing is listening on …".
                        Err(error) => error,
                    }
                }
            };

            guard.complete(output);
        });
    }
}

/// How long to wait for the downloader to die on exit before giving up.
/// Short: it is a well-behaved CLI, and a partial blob resumes next time.
const DOWNLOAD_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// How often to re-measure the bytes on disk while downloading.
const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// Runs `hf download`, streaming its output into the logs panel.
///
/// The CLI owns the hub cache layout — blobs, snapshot symlinks, refs —
/// which is the part that must not be got wrong, so it is left to do it.
async fn fetch(
    repo: &str,
    files: &[llama::hub::RepoFile],
    tx: &mpsc::UnboundedSender<UiEvent>,
    slot: &Arc<tokio::sync::Mutex<Option<tokio::process::Child>>>,
) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut child = tokio::process::Command::new(llama::hub::DOWNLOADER)
        .args(llama::hub::download_args(repo, files))
        // `hf` suppresses its progress entirely when stderr is not a
        // terminal unless this is set; without it the logs panel shows
        // nothing at all until the download ends.
        .env("HF_HUB_DISABLE_PROGRESS_BARS", "0")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            format!(
                "failed to run {}: {error} (is it on your PATH?)",
                llama::hub::DOWNLOADER
            )
        })?;

    let stderr = child.stderr.take();

    // Published before it is awaited, so a quit arriving mid-download has
    // something to kill. Taken back out below, so `shutdown` cannot end up
    // waiting on a process that has already been reaped here.
    *slot.lock().await = Some(child);

    if let Some(stderr) = stderr {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // tqdm redraws with carriage returns, so one "line" can
                // carry a whole download's worth of frames. Only the last
                // is current.
                if let Some(latest) = line.rsplit('\r').find(|part| !part.trim().is_empty()) {
                    let _ = tx.send(UiEvent::Log(latest.trim().to_string()));
                }
            }
        });
    }

    let Some(mut child) = slot.lock().await.take() else {
        // `shutdown` got there first and killed it.
        return Err(format!("{} was stopped", llama::hub::DOWNLOADER));
    };

    let status = child
        .wait()
        .await
        .map_err(|error| format!("{} did not finish: {error}", llama::hub::DOWNLOADER))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("{} exited with {status}", llama::hub::DOWNLOADER))
    }
}

/// Watches the bytes landing in the hub cache and reports them.
fn spawn_progress_poller(
    tx: mpsc::UnboundedSender<UiEvent>,
    model: String,
    repo: String,
    files: Vec<llama::hub::RepoFile>,
    total: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let Some(hub) = llama::hub::hub_dir() else {
            return;
        };
        let dir = llama::hub::repo_dir(&hub, &repo);

        loop {
            let done = llama::hub::downloaded_bytes(&dir, &files);
            if tx
                .send(UiEvent::DownloadProgress {
                    model: model.clone(),
                    done: done.min(total),
                    total,
                })
                .is_err()
            {
                return;
            }
            tokio::time::sleep(PROGRESS_INTERVAL).await;
        }
    })
}

fn send_error(
    tx: &mpsc::UnboundedSender<UiEvent>,
    mode: LauncherMode,
    model: Option<String>,
    error: &str,
) {
    let _ = tx.send(UiEvent::LlamaStatus(LlamaSnapshot::new(
        ServerState::Error(error.to_string()),
        mode,
        model,
    )));
}

/// Splits `"<model> -- <extra llama-server args>"` the same way the shell
/// would, without needing a shell: everything before a bare `--` token is
/// the model name, everything after is passed straight through as CLI
/// overrides.
fn split_extra_args(rest: &str) -> (String, Vec<String>) {
    match rest.split_once("--") {
        Some((model, extra)) => (
            model.trim().to_string(),
            extra.split_whitespace().map(str::to_string).collect(),
        ),
        None => (rest.trim().to_string(), Vec::new()),
    }
}

/// Parses `router [--max N] [--idle S]`, defaulting to the same values as
/// `startrouter.sh` (`--models-max 2 --sleep-idle-seconds 300`). Anything
/// else on the line is forwarded as extra llama-server flags.
fn parse_router_flags(rest: &str) -> (u32, u32, Vec<String>) {
    // The same defaults the Router screen starts from, so a typed
    // `:router` and an untouched screen cannot mean different things.
    let mut models_max = llama::prefs::DEFAULT_MODELS_MAX;
    let mut sleep_idle = llama::prefs::DEFAULT_SLEEP_IDLE_SECONDS;
    let mut extra = Vec::new();

    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "--max"
                if tokens
                    .get(i + 1)
                    .and_then(|v| v.parse::<u32>().ok())
                    .is_some() =>
            {
                models_max = tokens[i + 1].parse().unwrap();
                i += 2;
            }
            "--idle"
                if tokens
                    .get(i + 1)
                    .and_then(|v| v.parse::<u32>().ok())
                    .is_some() =>
            {
                sleep_idle = tokens[i + 1].parse().unwrap();
                i += 2;
            }
            other => {
                extra.push(other.to_string());
                i += 1;
            }
        }
    }

    (models_max, sleep_idle, extra)
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

/// Same contract as [`CompletionGuard`], for chat probes: the UI clears
/// its "in flight" flag on `ChatResult`, so exactly one must always be
/// sent — including if the task panics or is dropped mid-request.
struct ChatGuard {
    tx: mpsc::UnboundedSender<UiEvent>,
    done: bool,
}

impl ChatGuard {
    fn new(tx: mpsc::UnboundedSender<UiEvent>) -> Self {
        Self { tx, done: false }
    }

    fn complete(mut self, result: Result<llama::api::ChatOutcome, String>) {
        self.done = true;
        let _ = self.tx.send(UiEvent::ChatResult(Box::new(result)));
    }
}

impl Drop for ChatGuard {
    fn drop(&mut self) {
        if !self.done {
            let _ = self
                .tx
                .send(UiEvent::ChatResult(Box::new(Err("test aborted".into()))));
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

    /// These tests exercise dispatch paths that never read `models.ini`,
    /// so the path only has to be unambiguous, not real.
    fn no_config() -> PathBuf {
        PathBuf::from("/nonexistent/herd/models.ini")
    }

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
        let executor = Executor::new(tx, no_config());
        executor.run_command("help".into());

        match rx.recv().await.expect("event") {
            UiEvent::CommandFinished { command, .. } => {
                assert_eq!(command, "help");
            }
            other => panic!("expected CommandFinished, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn stop_with_nothing_running_reports_that() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let executor = Executor::new(tx, no_config());
        executor.run_command("stop".into());

        // Stopping nothing must not announce a STOPPING -> OFF transition:
        // the UI would flash a state change that never happened.
        match rx.recv().await.expect("event") {
            UiEvent::CommandFinished { command, output } => {
                assert_eq!(command, "stop");
                assert_eq!(output, "nothing was running");
            }
            other => panic!("expected CommandFinished only, got {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "no status event for a no-op stop");
    }

    /// The confirmation prompt re-dispatches as `launch!`, which must be
    /// recognised as a launch rather than falling through to the generic
    /// script path.
    #[tokio::test]
    async fn forced_launch_without_a_model_reports_usage() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let executor = Executor::new(tx, no_config());
        executor.run_command("launch!".into());

        match rx.recv().await.expect("event") {
            UiEvent::CommandFinished { output, .. } => {
                assert!(output.starts_with("usage:"), "got: {output}");
            }
            other => panic!("expected CommandFinished, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn launch_without_model_reports_usage() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let executor = Executor::new(tx, no_config());
        executor.run_command("launch".into());

        match rx.recv().await.expect("event") {
            UiEvent::CommandFinished { output, .. } => {
                assert!(output.starts_with("usage:"));
            }
            other => panic!("expected CommandFinished, got {:?}", other),
        }
    }

    /// Every command the listing shows must actually be dispatched.
    ///
    /// The half of the table that reaches the Executor, driven one at a
    /// time against a config that does not exist — so a launch reports a
    /// usage or a config error rather than spawning anything. "Unknown
    /// command" is what `run_script` says when nothing claimed the line,
    /// and it is the one answer a documented command may never give. This
    /// is the same bargain `every_key_that_does_something_is_documented`
    /// makes for the keymap: the listing is checked against the
    /// dispatcher, not merely written alongside it.
    #[tokio::test]
    async fn every_documented_command_reaches_a_handler() {
        use crate::commands::{Handler, ALL};

        for command in ALL.iter().filter(|c| c.handler != Handler::App) {
            let (tx, mut rx) = mpsc::unbounded_channel();
            let executor = Executor::new(tx, no_config());
            executor.run_command(command.probe.to_string());

            let output = loop {
                match rx.recv().await.expect("a completion") {
                    UiEvent::CommandFinished { output, .. } => break output,
                    // `cache` publishes its listing before completing.
                    _ => continue,
                }
            };

            assert_ne!(
                output, "Unknown command",
                "`:{}` is in the listing but nothing handles it",
                command.usage
            );
        }
    }

    /// The line this replaced, end to end:
    ///
    /// ```text
    /// :status -> http://127.0.0.1:1234 unreachable: request failed: error
    ///            sending request for url (http://127.0.0.1:1234/v1/models)
    /// ```
    ///
    /// Two problems in one line — it says "unreachable" and then explains
    /// it in reqwest's terms, and neither half tells the reader that the
    /// answer is to launch a model. Both `status` and `ping` reach the
    /// server the same way and were wrapped the same way.
    #[tokio::test]
    async fn status_and_ping_say_that_no_server_is_running() {
        // A real, closed port on the loopback interface: refused
        // immediately, no network involved.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().expect("addr").port();
            drop(listener);
            port
        };

        let config = std::env::temp_dir().join(format!(
            "herd-status-{}-{:?}.ini",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(
            &config,
            format!("[server]\nhost = 127.0.0.1\nport = {port}\n\n[a-model]\nhf-repo = v/m:Q4\n"),
        )
        .expect("write config");

        for command in ["status", "ping a-model"] {
            let (tx, mut rx) = mpsc::unbounded_channel();
            let executor = Executor::new(tx, config.clone());
            executor.run_command(command.to_string());

            let output = loop {
                match rx.recv().await.expect("a completion") {
                    UiEvent::CommandFinished { output, .. } => break output,
                    _ => continue,
                }
            };

            assert!(
                output.contains("no llama-server is running"),
                "`:{command}` is still not actionable: {output}"
            );
            assert!(
                !output.contains("error sending request"),
                "`:{command}` still leaks the plumbing: {output}"
            );
            // ...and it must not read as two problems stacked on each
            // other, which is what wrapping an explained error produced.
            assert!(
                !output.contains("unreachable:") && !output.contains("-> error:"),
                "`:{command}` wraps a message that already explains itself: {output}"
            );
        }

        let _ = std::fs::remove_file(&config);
    }

    #[test]
    fn split_extra_args_separates_model_and_overrides() {
        let (model, extra) = split_extra_args("gemma4-12b -- --ctx-size 65536");
        assert_eq!(model, "gemma4-12b");
        assert_eq!(extra, vec!["--ctx-size", "65536"]);
    }

    #[test]
    fn split_extra_args_without_double_dash() {
        let (model, extra) = split_extra_args("gemma4-12b");
        assert_eq!(model, "gemma4-12b");
        assert!(extra.is_empty());
    }

    #[test]
    fn parse_router_flags_defaults_match_startrouter_sh() {
        let (models_max, sleep_idle, extra) = parse_router_flags("");
        assert_eq!(models_max, 2);
        assert_eq!(sleep_idle, 300);
        assert!(extra.is_empty());
    }

    #[test]
    fn parse_router_flags_overrides() {
        let (models_max, sleep_idle, extra) = parse_router_flags("--max 3 --idle 120 --port 5555");
        assert_eq!(models_max, 3);
        assert_eq!(sleep_idle, 120);
        assert_eq!(extra, vec!["--port", "5555"]);
    }
}
