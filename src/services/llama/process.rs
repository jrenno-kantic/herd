//! Supervises a single `llama-server` child process (router or manual
//! mode) across command invocations: spawns it, streams its stdout/stderr
//! into the logs panel, drives the OFF/STARTING/SERVING/STOPPING/ERROR
//! state machine, and lets a later stop/launch kill it again.

use super::{api, LauncherMode, LlamaSnapshot, Phase, ServerState};
use crate::event::UiEvent;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};

pub const BINARY: &str = "llama-server";

/// How often to probe `/health` while STARTING.
const HEALTH_INTERVAL: Duration = Duration::from_millis(500);
/// How often to re-probe once the server is up. Slower than the startup
/// cadence — this is a liveness heartbeat, not a readiness race.
const SERVING_HEALTH_INTERVAL: Duration = Duration::from_secs(5);
/// How often to check whether the child has exited on its own.
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(200);
/// How long to keep probing before giving up on the model ever loading.
/// Generous: a cold 31B download-and-load can take minutes.
const HEALTH_BUDGET: Duration = Duration::from_secs(600);
/// How long the process may stay up without anything answering on the port
/// at all. Reaching this means the server never got as far as listening,
/// which is a different failure from a slow load and deserves a much
/// shorter leash than [`HEALTH_BUDGET`].
const BIND_BUDGET: Duration = Duration::from_secs(90);
/// Consecutive failed probes before a serving process is called
/// unresponsive. Three at the serving cadence is ~15s of silence, long
/// enough to ride out a single slow generation holding the event loop.
const UNRESPONSIVE_AFTER: u32 = 3;

/// How long to let the process shut down cleanly before escalating.
const TERM_GRACE: Duration = Duration::from_secs(5);
/// How long to wait for the kernel to finish reaping after SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(5);

/// What a [`Supervisor::stop`] actually achieved.
///
/// Distinguishing these matters because `Abandoned` is a real outcome on a
/// memory-tight machine: SIGKILL does not return until the kernel has torn
/// down the address space, and a process with tens of GiB mmap'd while the
/// system is swapping can take a long time to disappear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// Nothing was supervised.
    Nothing,
    /// Exited on SIGTERM, having had the chance to release its resources.
    Terminated,
    /// Ignored SIGTERM; exited after SIGKILL.
    Killed,
    /// Still had not exited when the grace ran out. The handle is dropped,
    /// so `kill_on_drop` remains as a backstop, but we stop waiting.
    Abandoned,
}

impl Stopped {
    pub fn label(self) -> &'static str {
        match self {
            Stopped::Nothing => "nothing was running",
            Stopped::Terminated => "llama-server stopped",
            Stopped::Killed => "llama-server killed (it ignored SIGTERM)",
            Stopped::Abandoned => {
                "llama-server did not exit in time — it is still shutting down in the background"
            }
        }
    }
}

/// The supervised process plus the launch it belongs to.
///
/// The generation is what stops a watcher from the *previous* launch
/// acting on the current one: `stop()` and a hot-swap both empty and refill
/// the slot, so "is there a child?" alone cannot tell an old task whether
/// the child it sees is still the one it was spawned for.
struct Supervised {
    child: Child,
    generation: u64,
}

/// Cheap to clone — every clone shares the same underlying child handle,
/// so `Executor` can hand out clones freely while keeping one canonical
/// "currently supervised process" slot alive for the lifetime of the app.
#[derive(Clone, Default)]
pub struct Supervisor {
    child: Arc<Mutex<Option<Supervised>>>,
    launches: Arc<AtomicU64>,
    logs: Option<crate::event::LogSender>,
}

impl Supervisor {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_logs(logs: Option<crate::event::LogSender>) -> Self {
        Self {
            logs,
            ..Self::default()
        }
    }

    /// Stop whatever is currently supervised, if anything.
    ///
    /// Two-phase and **bounded at every step**. SIGTERM first, because
    /// llama-server handles it and releases the GPU allocation on the way
    /// out, where SIGKILL leaves that to the kernel. Then SIGKILL, then —
    /// crucially — giving up rather than waiting forever.
    ///
    /// The unbounded `wait()` this replaces was on the critical path of
    /// three separate things: every hot-swap (`spawn` stops first), the
    /// `:stop` command, and `shutdown()` on quit. A kill that takes twenty
    /// seconds to be reaped therefore froze the UI on STOPPING, made the
    /// app appear hung on quit, and — because the busy flag was still set —
    /// locked the user out of every other command meanwhile.
    ///
    /// The child is taken *out* of the slot before being awaited, so the
    /// mutex is never held across an await. See [`spawn_exit_watcher`] for
    /// why that rule is load-bearing here.
    pub async fn stop(&self) -> Stopped {
        let taken = self.child.lock().await.take();

        let Some(mut supervised) = taken else {
            return Stopped::Nothing;
        };

        if terminate(&supervised.child) && wait_for(&mut supervised.child, TERM_GRACE).await {
            return Stopped::Terminated;
        }

        let _ = supervised.child.start_kill();
        if wait_for(&mut supervised.child, KILL_GRACE).await {
            Stopped::Killed
        } else {
            // Dropping the handle leaves `kill_on_drop` and tokio's reaper
            // to finish the job; what we must not do is keep blocking.
            Stopped::Abandoned
        }
    }

    /// Same as [`Supervisor::stop`], but announces STOPPING first and OFF
    /// afterwards so the UI shows the transition instead of jumping
    /// straight from SERVING to OFF.
    pub async fn stop_announced(&self, tx: &mpsc::UnboundedSender<UiEvent>) -> Stopped {
        if !self.is_running().await {
            return Stopped::Nothing;
        }

        let _ = tx.send(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Stopping,
            LauncherMode::Idle,
            None,
        )));

        let stopped = self.stop().await;

        let _ = tx.send(UiEvent::LlamaStatus(LlamaSnapshot::off()));
        stopped
    }

    pub async fn is_running(&self) -> bool {
        self.child.lock().await.is_some()
    }

    /// Spawn `llama-server` with the given argv, stopping any previously
    /// supervised process first (hot-swap). Streams stdout/stderr line by
    /// line as `UiEvent::Log`, and drives the state machine: STARTING on
    /// spawn, SERVING once `/health` returns 200, ERROR on a non-zero exit.
    pub async fn spawn(
        &self,
        mode: LauncherMode,
        model: Option<String>,
        args: Vec<String>,
        base_url: String,
        tx: mpsc::UnboundedSender<UiEvent>,
        repo: Option<String>,
    ) -> Result<(), String> {
        self.spawn_program(BINARY, mode, model, args, base_url, tx, repo)
            .await
    }

    /// Same as [`Supervisor::spawn`] with the program name injected, so
    /// tests can supervise something cheap and predictable instead of a
    /// real llama-server.
    #[allow(clippy::too_many_arguments)]
    async fn spawn_program(
        &self,
        program: &str,
        mode: LauncherMode,
        model: Option<String>,
        args: Vec<String>,
        base_url: String,
        tx: mpsc::UnboundedSender<UiEvent>,
        repo: Option<String>,
    ) -> Result<(), String> {
        // A hot-swap stops the previous process first, and with a big
        // model paging out that stop takes seconds to tens of seconds.
        // Doing it silently left the UI frozen on SERVING for the whole
        // wait — pressing enter on the Router screen with a model up
        // looked exactly like a hang. Announce it, but only when there is
        // something to stop: flashing STOPPING on a cold start would be a
        // transition that never happened (same rule as `stop_announced`).
        if self.is_running().await {
            let _ = tx.send(UiEvent::LlamaStatus(LlamaSnapshot::new(
                ServerState::Stopping,
                mode,
                None,
            )));
        }

        if let Some(reason) = swap_refusal(self.stop().await) {
            return Err(reason);
        }

        let _ = tx.send(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Starting,
            mode,
            model.clone(),
        )));
        let _ = tx.send(UiEvent::Log(format!("$ {program} {}", args.join(" "))));

        let mut command = Command::new(program);
        command
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to spawn {program}: {error} (is it on your PATH?)"))?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let generation = self.launches.fetch_add(1, Ordering::SeqCst) + 1;
        *self.child.lock().await = Some(Supervised { child, generation });

        if let Some(stdout) = stdout {
            spawn_line_forwarder(stdout, tx.clone(), self.logs.clone());
        }
        if let Some(stderr) = stderr {
            spawn_line_forwarder(stderr, tx.clone(), self.logs.clone());
        }

        spawn_health_poller(
            self.child.clone(),
            generation,
            base_url,
            tx.clone(),
            mode,
            model.clone(),
            repo,
        );
        spawn_exit_watcher(self.child.clone(), generation, tx, mode, model);

        Ok(())
    }
}

/// Whether a hot-swap may proceed after stopping the previous process.
///
/// An `Abandoned` stop means the predecessor is still tearing down and
/// still holds its port — spawning into it guarantees llama-server's raw
/// "couldn't bind HTTP server socket" a moment later, which is the least
/// useful way to learn what happened. Refusing with the actual cause turns
/// a confusing bind error into an instruction.
fn swap_refusal(stopped: Stopped) -> Option<String> {
    match stopped {
        Stopped::Abandoned => Some(format!(
            "the previous {BINARY} is still shutting down and holds the port — \
             try again in a few seconds"
        )),
        Stopped::Nothing | Stopped::Terminated | Stopped::Killed => None,
    }
}

/// Is the slot still holding the launch this task was spawned for?
///
/// Answering with "is there any child?" would let a task from a previous
/// launch act on the current one after a stop-then-launch or a hot-swap,
/// reporting the new process under the old model's name.
async fn is_current(child_slot: &Arc<Mutex<Option<Supervised>>>, generation: u64) -> bool {
    child_slot
        .lock()
        .await
        .as_ref()
        .is_some_and(|supervised| supervised.generation == generation)
}

/// What one health probe should cause. Empty by default: the common case
/// is a probe that confirms what is already on screen and must produce
/// nothing at all, or the UI would redraw itself every few hundred
/// milliseconds for the life of the server.
#[derive(Debug, Default, PartialEq, Eq)]
struct Report {
    /// A lifecycle change to broadcast.
    status: Option<(ServerState, Phase)>,
    /// A one-off line for the logs panel, emitted only on the edges.
    log: Option<String>,
    /// Stop polling: the verdict is final.
    done: bool,
}

impl Report {
    fn status(state: ServerState, phase: Phase) -> Self {
        Self {
            status: Some((state, phase)),
            ..Self::default()
        }
    }

    fn logged(mut self, line: impl Into<String>) -> Self {
        self.log = Some(line.into());
        self
    }

    fn final_status(mut self) -> Self {
        self.done = true;
        self
    }
}

/// The health poller's decision logic, separated from the polling so it can
/// be tested a probe at a time instead of in real seconds.
#[derive(Debug, Default)]
struct HealthTracker {
    serving: bool,
    reported: Phase,
    misses: u32,
    /// Partial bytes seen last time, and when they last grew. The budgets
    /// run from the last sign of progress rather than from the launch, so
    /// a download cannot time out merely for being large.
    partial: u64,
    progress_at: Duration,
}

impl HealthTracker {
    /// How long to wait before the next probe. Fast while racing to find
    /// the ready edge, slow once this is only a liveness heartbeat.
    fn interval(&self) -> Duration {
        if self.serving {
            SERVING_HEALTH_INTERVAL
        } else {
            HEALTH_INTERVAL
        }
    }

    /// Folds one probe into the running verdict.
    ///
    /// `elapsed` is time since the launch, supplied by the caller so this
    /// stays a pure function of its inputs.
    fn observe(
        &mut self,
        health: api::Health,
        elapsed: Duration,
        base_url: &str,
        partial: u64,
    ) -> Report {
        if self.serving {
            return self.observe_while_serving(health);
        }

        // Weights arriving is progress, even though the port is silent.
        if partial > self.partial {
            self.partial = partial;
            self.progress_at = elapsed;
        }
        let stalled_for = elapsed.saturating_sub(self.progress_at);

        match health {
            api::Health::Serving => {
                self.serving = true;
                self.misses = 0;
                self.reported = Phase::None;
                Report::status(ServerState::Serving, Phase::None)
            }
            // 503: the port is bound and llama-server is loading. That it
            // answered at all is what retires the bind budget.
            api::Health::Loading => {
                if self.reported == Phase::Loading {
                    return Report::default();
                }
                self.reported = Phase::Loading;
                Report::status(ServerState::Starting, Phase::Loading)
            }
            api::Health::Unreachable => {
                // A download in flight is not a failure to bind. It is the
                // one silent-port state that is entirely healthy, and it
                // can legitimately run for half an hour.
                if self.partial > 0 {
                    // ...as long as it is still arriving. A partial that
                    // has not grown in the whole load budget is a dead
                    // download, and waiting on it forever is the very
                    // silence this phase exists to break.
                    if stalled_for >= HEALTH_BUDGET {
                        return Report::status(
                            ServerState::Error(format!(
                                "download stalled at {} after {}s with no new bytes",
                                super::hub::human_bytes(self.partial),
                                stalled_for.as_secs()
                            )),
                            Phase::None,
                        )
                        .final_status();
                    }

                    let phase = Phase::Downloading(self.partial);
                    if self.reported != phase {
                        self.reported = phase;
                        return Report::status(ServerState::Starting, phase);
                    }
                    return Report::default();
                }

                // Never having listened is a different failure from loading
                // slowly, and gets a much shorter leash: ten minutes of
                // STARTING for a server that never opened its port is ten
                // minutes of the UI implying something is still happening.
                if self.reported == Phase::Binding && stalled_for >= BIND_BUDGET {
                    return Report::status(
                        ServerState::Error(format!(
                            "nothing is listening on {base_url} after {}s",
                            BIND_BUDGET.as_secs()
                        )),
                        Phase::None,
                    )
                    .final_status();
                }
                if self.reported != Phase::Binding {
                    self.reported = Phase::Binding;
                    return Report::status(ServerState::Starting, Phase::Binding);
                }
                if stalled_for >= HEALTH_BUDGET {
                    return Report::status(
                        ServerState::Error(format!(
                            "not serving after {}s",
                            HEALTH_BUDGET.as_secs()
                        )),
                        Phase::None,
                    )
                    .final_status();
                }
                Report::default()
            }
        }
    }

    fn observe_while_serving(&mut self, health: api::Health) -> Report {
        if health == api::Health::Serving {
            self.misses = 0;
            if !self.reported.is_degraded() {
                return Report::default();
            }
            self.reported = Phase::None;
            return Report::status(ServerState::Serving, Phase::None)
                .logged(format!("{BINARY} is answering /health again"));
        }

        // Alive but silent. Deliberately never escalated to `Error`: the
        // process is still there and may well recover, and the exit watcher
        // owns the "it died" verdict. Saying SERVING unqualified, though,
        // would be a plain lie.
        self.misses += 1;
        if self.misses < UNRESPONSIVE_AFTER {
            return Report::default();
        }

        let first = !self.reported.is_degraded();
        self.reported = Phase::Unresponsive(self.misses);
        let report = Report::status(ServerState::Serving, self.reported);

        if first {
            report.logged(format!(
                "{BINARY} stopped answering /health while still running \
                 (paging, or a request holding the server?)"
            ))
        } else {
            report
        }
    }
}

/// Polls `/health` for the whole life of a launch: first to find the
/// STARTING -> SERVING edge, then as a liveness heartbeat.
///
/// **It deliberately does not stop at the first 200.** A supervised
/// process that stops answering while staying alive is invisible to the
/// exit watcher, so a poller that retired on success left the UI reporting
/// SERVING for a server that had gone silent — the single most misleading
/// thing this program could display. On a machine whose RAM is close to
/// the model size that is not a corner case: the weights get paged out
/// from under the server and it stalls without ever dying.
///
/// The child check remains load-bearing in the other direction: without it
/// a model that dies during load would leave the UI stuck on STARTING.
#[allow(clippy::too_many_arguments)]
fn spawn_health_poller(
    child_slot: Arc<Mutex<Option<Supervised>>>,
    generation: u64,
    base_url: String,
    tx: mpsc::UnboundedSender<UiEvent>,
    mode: LauncherMode,
    model: Option<String>,
    repo: Option<String>,
) {
    tokio::spawn(async move {
        let started = tokio::time::Instant::now();
        let mut tracker = HealthTracker::default();

        // What was already half-downloaded before this launch.
        //
        // A killed download leaves its partial behind — this cache holds
        // several — and counting those as progress would report
        // "downloading 192M" for a launch where nothing is being fetched,
        // and hold off the bind budget for ten minutes on the strength of
        // bytes that arrived days ago. Only growth beyond this counts, so
        // a resumed download still reads correctly: it grows from here.
        let baseline = repo
            .as_deref()
            .map(super::hub::partial_bytes_for)
            .unwrap_or(0);

        loop {
            tokio::time::sleep(tracker.interval()).await;

            if !is_current(&child_slot, generation).await {
                return; // stopped, hot-swapped, or exited: not our business
            }

            let health = api::health(&base_url).await;
            // Measured, not parsed — the same reasoning as the download
            // bar. llama-server writes its partial into the hub cache, and
            // watching it grow says "still working" without depending on
            // anything llama.cpp prints.
            let partial = repo
                .as_deref()
                .map(super::hub::partial_bytes_for)
                .unwrap_or(0)
                .saturating_sub(baseline);
            let report = tracker.observe(health, started.elapsed(), &base_url, partial);

            // Checked again, because probing is not instant: a `/health`
            // call waits out its timeout, and a stop landing in that
            // window has already announced OFF by the time we get here.
            // Emitting now would put STARTING — or, mid-download, an error
            // about nothing listening — on top of it, leaving the app
            // showing a failed server that is not even running.
            if !is_current(&child_slot, generation).await {
                return;
            }

            if let Some(line) = report.log {
                let _ = tx.send(UiEvent::Log(line));
            }
            if let Some((state, phase)) = report.status {
                let _ = tx.send(UiEvent::LlamaStatus(
                    LlamaSnapshot::new(state, mode, model.clone()).with_phase(phase),
                ));
            }
            if report.done {
                return;
            }
        }
    });
}

/// Watches for the child ending on its own.
///
/// **Polls with `try_wait` instead of awaiting `wait`.** The child lives
/// behind the shared mutex so that a concurrent `stop()` is the single
/// source of truth for "is a process still supervised" — but that means
/// awaiting `child.wait()` while holding the guard would pin the lock for
/// the entire lifetime of the process. Everything else that needs it (the
/// `/health` poller, `is_running`, `stop`) would block forever, leaving
/// the UI stuck on STARTING with a server that is actually up. Each lock
/// taken here must be released within the same iteration.
fn spawn_exit_watcher(
    child_slot: Arc<Mutex<Option<Supervised>>>,
    generation: u64,
    tx: mpsc::UnboundedSender<UiEvent>,
    mode: LauncherMode,
    model: Option<String>,
) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(EXIT_POLL_INTERVAL).await;

            let exited = {
                let mut guard = child_slot.lock().await;
                match guard.as_mut() {
                    // Taken by `stop()`, or already replaced by a newer
                    // launch: either way this task is no longer relevant.
                    Some(supervised) if supervised.generation != generation => return,
                    None => return,
                    // `try_wait` returns an owned status, so the borrow of
                    // the guard ends here and the slot can be cleared.
                    Some(supervised) => match supervised.child.try_wait() {
                        Ok(Some(status)) => Some(Some(status)),
                        Ok(None) => None,
                        Err(_) => Some(None),
                    },
                }
            };

            let Some(status) = exited else {
                continue;
            };

            {
                let mut guard = child_slot.lock().await;
                if guard.as_ref().is_some_and(|s| s.generation == generation) {
                    guard.take();
                }
            }

            let state = match status {
                Some(status) => {
                    let _ = tx.send(UiEvent::Log(format!("{BINARY} exited: {status}")));
                    diagnose(status)
                }
                None => ServerState::Error("could not read the exit status".into()),
            };

            let _ = tx.send(UiEvent::LlamaStatus(LlamaSnapshot::new(state, mode, model)));
            return;
        }
    });
}

/// Turns an exit status into something the user can act on.
///
/// A bare "exited with signal: 9" is the least useful thing this program
/// could say about its most common failure. Anything reaching here died
/// *without* herd asking — `stop()` takes the child out of the slot
/// before killing it, so the watcher has already retired by then — which
/// means a fatal signal is somebody else's doing. On a machine that is
/// short of memory for the model, that somebody is almost always the
/// kernel reclaiming it.
fn diagnose(status: ExitStatus) -> ServerState {
    #[cfg(unix)]
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
        return ServerState::Error(match signal {
            libc::SIGKILL => "killed by the system (SIGKILL) — most likely out of memory".into(),
            libc::SIGABRT => {
                "aborted (SIGABRT) — llama.cpp aborts here when a backend allocation fails".into()
            }
            libc::SIGSEGV | libc::SIGBUS => format!("crashed (signal {signal})"),
            other => format!("killed by signal {other}"),
        });
    }

    if status.success() {
        ServerState::Off
    } else {
        ServerState::Error(format!("exited with {status}"))
    }
}

/// Asks the process to exit, returning whether the signal was delivered.
///
/// llama-server releases its GPU allocation on SIGTERM; `start_kill` sends
/// SIGKILL, which does not give it that chance.
#[cfg(unix)]
fn terminate(child: &Child) -> bool {
    match child.id() {
        // Safety: `kill` on a pid we own; the worst case of a raced pid is
        // an ESRCH we already tolerate via the return value.
        Some(pid) => unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) == 0 },
        None => false,
    }
}

#[cfg(not(unix))]
fn terminate(_child: &Child) -> bool {
    false
}

/// Waits for the child, giving up after `grace`. Returns whether it is
/// gone — a `wait` that errors counts, since the only way to fail here is
/// for the child to be unwaitable, which means it is no longer ours to
/// wait for. Only the timeout means "still running".
async fn wait_for(child: &mut Child, grace: Duration) -> bool {
    tokio::time::timeout(grace, child.wait()).await.is_ok()
}

fn spawn_line_forwarder<R>(
    reader: R,
    tx: mpsc::UnboundedSender<UiEvent>,
    logs: Option<crate::event::LogSender>,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            match &logs {
                Some(logs) => logs.send(line),
                None if tx.send(UiEvent::Log(line)).is_err() => break,
                None => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stop_without_running_process_is_false() {
        let supervisor = Supervisor::new();
        assert_eq!(supervisor.stop().await, Stopped::Nothing);
    }

    /// `stop_announced` must not emit STOPPING/OFF when there was nothing
    /// to stop — otherwise pressing stop on an idle server would flash a
    /// transition that never happened.
    #[tokio::test]
    async fn stop_announced_on_idle_emits_nothing() {
        let supervisor = Supervisor::new();
        let (tx, mut rx) = mpsc::unbounded_channel();

        assert_eq!(supervisor.stop_announced(&tx).await, Stopped::Nothing);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn is_running_is_false_before_any_spawn() {
        assert!(!Supervisor::new().is_running().await);
    }

    async fn spawn_sleeper(supervisor: &Supervisor) -> mpsc::UnboundedReceiver<UiEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        supervisor
            .spawn_program(
                "/bin/sleep",
                LauncherMode::Manual,
                Some("sleeper".into()),
                vec!["30".into()],
                // Nothing listens here: the health poller must keep
                // probing rather than ever reporting Serving.
                "http://127.0.0.1:1".into(),
                tx,
                None,
            )
            .await
            .expect("spawn /bin/sleep");
        rx
    }

    /// Regression test for a deadlock that pinned the UI on STARTING: the
    /// exit watcher used to hold the child mutex across `child.wait()`,
    /// i.e. for the entire lifetime of the process. Everything else that
    /// needs the lock — the `/health` poller, `is_running`, `stop` — then
    /// blocked forever, so the server never left STARTING and `:stop`
    /// hung. Every lock here must be released promptly.
    #[tokio::test]
    async fn a_live_child_never_holds_the_lock() {
        let supervisor = Supervisor::new();
        let _rx = spawn_sleeper(&supervisor).await;

        // Give the background watchers a chance to run. Without this the
        // test races them and passes even when the deadlock is present:
        // in the real app the event loop yields constantly, so the watcher
        // has always taken the lock by the time a key is pressed.
        tokio::time::sleep(Duration::from_millis(300)).await;

        let running = tokio::time::timeout(Duration::from_secs(2), supervisor.is_running()).await;
        assert_eq!(
            running,
            Ok(true),
            "is_running blocked: the child lock is held for the process lifetime"
        );

        let stopped = tokio::time::timeout(Duration::from_secs(5), supervisor.stop()).await;
        assert_eq!(
            stopped,
            Ok(Stopped::Terminated),
            "stop blocked on the child lock"
        );
        assert!(!supervisor.is_running().await);
    }

    /// End-to-end proof that a supervised process which starts answering
    /// `/health` moves STARTING -> SERVING. This is the exact path that
    /// was broken by the lock deadlock: the server was up and healthy, but
    /// the UI sat on STARTING indefinitely.
    ///
    /// Uses a tiny always-200 HTTP server rather than a real llama-server,
    /// so it costs no GPU and no model download. Skips if python3 is absent.
    #[tokio::test]
    async fn a_healthy_process_transitions_to_serving() {
        const PORT: u16 = 18234;
        const SCRIPT: &str = "\
import http.server, sys
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b'{\"status\":\"ok\"}')
    def log_message(self, *a): pass
http.server.HTTPServer(('127.0.0.1', 18234), H).serve_forever()
";

        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("skipping: python3 not available");
            return;
        }

        let supervisor = Supervisor::new();
        let (tx, mut rx) = mpsc::unbounded_channel();

        supervisor
            .spawn_program(
                "python3",
                LauncherMode::Manual,
                Some("fake".into()),
                vec!["-c".into(), SCRIPT.into()],
                format!("http://127.0.0.1:{PORT}"),
                tx,
                None,
            )
            .await
            .expect("spawn python3");

        let states = tokio::time::timeout(Duration::from_secs(15), async {
            let mut seen = Vec::new();
            while let Some(event) = rx.recv().await {
                if let UiEvent::LlamaStatus(snapshot) = event {
                    let state = snapshot.state.clone();
                    seen.push(state.clone());
                    if state == ServerState::Serving {
                        return seen;
                    }
                }
            }
            seen
        })
        .await;

        supervisor.stop().await;

        let states = states.expect("timed out waiting for SERVING");
        assert_eq!(states.first(), Some(&ServerState::Starting));
        assert_eq!(states.last(), Some(&ServerState::Serving));
    }

    fn always_ok_server(port: u16) -> Vec<String> {
        vec![
            "-c".into(),
            format!(
                "\
import http.server
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b'{{\"status\":\"ok\"}}')
    def log_message(self, *a): pass
http.server.HTTPServer(('127.0.0.1', {port}), H).serve_forever()
"
            ),
        ]
    }

    /// Drains status events into an `App` until it reaches `target`, so a
    /// test can assert on what the *UI* ends up showing rather than on the
    /// raw event stream.
    async fn drive_until(
        app: &mut crate::app::App,
        rx: &mut mpsc::UnboundedReceiver<UiEvent>,
        target: ServerState,
    ) -> bool {
        tokio::time::timeout(Duration::from_secs(15), async {
            while let Some(event) = rx.recv().await {
                let is_status = matches!(event, UiEvent::LlamaStatus(_));
                app.update(event);
                if is_status && app.llama.server.state == target {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false)
    }

    /// Reproduces the reported sequence: launch a model, stop it, launch
    /// the next one. The UI must end up SERVING the *second* model, and
    /// must not keep showing the first one as the active model — that is
    /// what leaves a "currently serving" dot next to the wrong row.
    #[tokio::test]
    async fn stopping_then_launching_another_model_switches_cleanly() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("skipping: python3 not available");
            return;
        }

        let mut app =
            crate::app::App::with_config_path(std::path::PathBuf::from("/nonexistent/models.ini"));
        let supervisor = Supervisor::new();
        let (tx, mut rx) = mpsc::unbounded_channel();

        // 1. first model comes up
        supervisor
            .spawn_program(
                "python3",
                LauncherMode::Manual,
                Some("gemma4-12b".into()),
                always_ok_server(18241),
                "http://127.0.0.1:18241".into(),
                tx.clone(),
                None,
            )
            .await
            .expect("spawn first");
        assert!(
            drive_until(&mut app, &mut rx, ServerState::Serving).await,
            "first model never reached SERVING"
        );
        assert_eq!(app.llama.server.model.as_deref(), Some("gemma4-12b"));

        // 2. stop it
        supervisor.stop_announced(&tx).await;
        assert!(
            drive_until(&mut app, &mut rx, ServerState::Off).await,
            "stop never reached OFF"
        );
        assert_eq!(
            app.llama.server.model, None,
            "the stopped model is still shown as active: its row keeps the serving dot"
        );

        // 3. launch the next one
        supervisor
            .spawn_program(
                "python3",
                LauncherMode::Manual,
                Some("qwen3-coder".into()),
                always_ok_server(18242),
                "http://127.0.0.1:18242".into(),
                tx.clone(),
                None,
            )
            .await
            .expect("spawn second");
        assert!(
            drive_until(&mut app, &mut rx, ServerState::Serving).await,
            "second model never left STARTING"
        );

        assert_eq!(app.llama.server.model.as_deref(), Some("qwen3-coder"));
        supervisor.stop().await;
    }

    /// Pressing stop while a launch is still probing must leave the app
    /// OFF, not back in STARTING or ERROR.
    ///
    /// The window is real: the poller checks whether the launch is still
    /// current, then spends up to the health timeout waiting on a reply.
    /// A stop completing inside that window emits OFF, and the poller then
    /// overwrote it with whatever it had computed — which during a
    /// download is an error about nothing listening. The app sat in ERROR
    /// with no running process and no way to clear it.
    ///
    /// Held open deliberately with a listener that accepts and never
    /// answers, so the race is not left to chance.
    #[tokio::test]
    async fn a_stop_during_a_probe_leaves_the_app_off() {
        const PORT: u16 = 18251;
        const HANGS: &str = "\
import socket
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(('127.0.0.1', 18251)); s.listen(8)
conns = []
while True:
    c, _ = s.accept()
    conns.append(c)   # accepted, never answered
";

        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("skipping: python3 not available");
            return;
        }

        let mut hang = tokio::process::Command::new("python3")
            .args(["-c", HANGS])
            .kill_on_drop(true)
            .spawn()
            .expect("spawn the hanging listener");
        tokio::time::sleep(Duration::from_millis(400)).await;

        let supervisor = Supervisor::new();
        let (tx, mut rx) = mpsc::unbounded_channel();

        supervisor
            .spawn_program(
                "/bin/sleep",
                LauncherMode::Manual,
                Some("stalling".into()),
                vec!["30".into()],
                format!("http://127.0.0.1:{PORT}"),
                tx.clone(),
                None,
            )
            .await
            .expect("spawn");

        // Long enough for the poller to be inside the probe it will never
        // get an answer to.
        tokio::time::sleep(Duration::from_millis(900)).await;
        supervisor.stop_announced(&tx).await;

        // Past the health timeout, so a late emit would have landed.
        tokio::time::sleep(Duration::from_secs(4)).await;
        let _ = hang.kill().await;

        let mut states = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let UiEvent::LlamaStatus(snapshot) = event {
                states.push(snapshot.state);
            }
        }

        assert_eq!(
            states.last(),
            Some(&ServerState::Off),
            "a retired poller reported after the stop: {states:?}"
        );
    }

    /// A hot-swap must say it is stopping the previous process. The stop
    /// is bounded but not instant — a big model paging out takes seconds
    /// to tens of seconds to die — and doing it silently left the UI
    /// frozen on SERVING for the whole wait, which is exactly what "the
    /// router hangs to start when a model is already serving" looks like.
    #[tokio::test]
    async fn a_hot_swap_announces_the_stop() {
        let supervisor = Supervisor::new();
        let (tx, mut rx) = mpsc::unbounded_channel();

        supervisor
            .spawn_program(
                "/bin/sleep",
                LauncherMode::Manual,
                Some("first".into()),
                vec!["30".into()],
                "http://127.0.0.1:1".into(),
                tx.clone(),
                None,
            )
            .await
            .expect("spawn first");

        supervisor
            .spawn_program(
                "/bin/sleep",
                LauncherMode::Router,
                None,
                vec!["30".into()],
                "http://127.0.0.1:1".into(),
                tx.clone(),
                None,
            )
            .await
            .expect("spawn second");
        supervisor.stop().await;

        let mut states = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let UiEvent::LlamaStatus(snapshot) = event {
                // Only the lifecycle events the spawns emit themselves: a
                // health poller that got a probe in before the drain adds
                // phase-carrying snapshots this test is not about.
                if snapshot.phase == Phase::None {
                    states.push((snapshot.state, snapshot.mode));
                }
            }
        }

        assert_eq!(
            states,
            vec![
                (ServerState::Starting, LauncherMode::Manual),
                // The swap: stopping, in the mode being started, so the
                // screen the user pressed enter on shows it immediately.
                (ServerState::Stopping, LauncherMode::Router),
                (ServerState::Starting, LauncherMode::Router),
            ],
            "the swap happened without saying so"
        );
    }

    /// ...but a spawn with nothing running must not flash STOPPING — the
    /// same rule as `stop_announced`: never show a transition that did
    /// not happen.
    #[tokio::test]
    async fn a_cold_spawn_does_not_flash_stopping() {
        let supervisor = Supervisor::new();
        let mut rx = spawn_sleeper(&supervisor).await;
        supervisor.stop().await;

        while let Ok(event) = rx.try_recv() {
            if let UiEvent::LlamaStatus(snapshot) = event {
                assert_ne!(
                    snapshot.state,
                    ServerState::Stopping,
                    "a cold start announced a stop that never happened"
                );
            }
        }
    }

    /// A stop that ran out its grace leaves the predecessor alive and
    /// still holding the port. Spawning into that guarantees a raw bind
    /// error moments later; the refusal has to name the actual cause.
    #[test]
    fn an_abandoned_stop_refuses_the_swap() {
        let refusal = swap_refusal(Stopped::Abandoned).expect("must refuse");
        assert!(refusal.contains("shutting down"), "{refusal}");

        for stopped in [Stopped::Nothing, Stopped::Terminated, Stopped::Killed] {
            assert_eq!(swap_refusal(stopped), None, "{stopped:?} blocked a swap");
        }
    }

    /// Hot-swapping replaces the child in the shared slot. The watchers
    /// from the previous launch must recognise that the child they now see
    /// is not theirs and bow out, instead of reporting the new process
    /// under the old model's name.
    #[tokio::test]
    async fn a_hot_swap_retires_the_previous_launch_watchers() {
        let supervisor = Supervisor::new();
        let (tx, mut rx) = mpsc::unbounded_channel();

        // A long-lived first launch, then immediately swap to a short one.
        supervisor
            .spawn_program(
                "/bin/sleep",
                LauncherMode::Manual,
                Some("first".into()),
                vec!["30".into()],
                "http://127.0.0.1:1".into(),
                tx.clone(),
                None,
            )
            .await
            .expect("spawn first");

        supervisor
            .spawn_program(
                "/bin/sleep",
                LauncherMode::Manual,
                Some("second".into()),
                vec!["0.2".into()],
                "http://127.0.0.1:1".into(),
                tx.clone(),
                None,
            )
            .await
            .expect("spawn second");

        // Collect terminal events for a while after the second exits.
        let terminal = tokio::time::timeout(Duration::from_secs(5), async {
            let mut seen = Vec::new();
            while let Some(event) = rx.recv().await {
                if let UiEvent::LlamaStatus(snapshot) = event {
                    if !snapshot.state.is_live() {
                        seen.push(snapshot.model.clone());
                        // Give any stale watcher a chance to also fire.
                        tokio::time::sleep(Duration::from_millis(600)).await;
                        return seen;
                    }
                }
            }
            seen
        })
        .await
        .expect("timed out");

        assert_eq!(
            terminal,
            vec![Some("second".to_string())],
            "a retired watcher reported the new process"
        );
        assert!(!supervisor.is_running().await);
    }

    /// The exit watcher must notice a process that ends on its own and
    /// report it, without anyone calling `stop`.
    #[tokio::test]
    async fn a_child_that_exits_on_its_own_is_reported() {
        let supervisor = Supervisor::new();
        let (tx, mut rx) = mpsc::unbounded_channel();

        supervisor
            .spawn_program(
                "/bin/sleep",
                LauncherMode::Manual,
                Some("brief".into()),
                vec!["0.2".into()],
                "http://127.0.0.1:1".into(),
                tx,
                None,
            )
            .await
            .expect("spawn");

        let saw_off = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = rx.recv().await {
                if let UiEvent::LlamaStatus(snapshot) = event {
                    if snapshot.state == ServerState::Off {
                        return true;
                    }
                }
            }
            false
        })
        .await;

        assert_eq!(saw_off, Ok(true), "clean exit never reported as Off");
        assert!(!supervisor.is_running().await);
    }

    /// The whole point of the rewritten poller: reaching SERVING must not
    /// end the watch. A server that goes silent while its process stays
    /// alive is invisible to the exit watcher, so if the poller retires
    /// here nothing will ever contradict the SERVING on screen.
    #[test]
    fn a_serving_server_that_goes_silent_stops_being_called_healthy() {
        let mut tracker = HealthTracker::default();
        let url = "http://127.0.0.1:1234";

        assert_eq!(
            tracker
                .observe(api::Health::Serving, Duration::ZERO, url, 0)
                .status,
            Some((ServerState::Serving, Phase::None))
        );

        // Silence, but not yet enough of it to call: a single missed probe
        // is a slow request, not a stall.
        for _ in 1..UNRESPONSIVE_AFTER {
            let report = tracker.observe(api::Health::Unreachable, Duration::ZERO, url, 0);
            assert_eq!(report.status, None, "called it dead on the first miss");
        }

        let report = tracker.observe(api::Health::Unreachable, Duration::ZERO, url, 0);
        assert_eq!(
            report.status,
            Some((
                ServerState::Serving,
                Phase::Unresponsive(UNRESPONSIVE_AFTER)
            ))
        );
        assert!(report.log.is_some(), "the stall was never logged");
        assert!(!report.done, "a stalled server may still recover");
    }

    /// ...and when it does answer again, the warning has to clear, or the
    /// UI cries wolf for the rest of the session.
    #[test]
    fn a_recovered_server_clears_the_warning() {
        let mut tracker = HealthTracker::default();
        let url = "http://127.0.0.1:1234";

        tracker.observe(api::Health::Serving, Duration::ZERO, url, 0);
        for _ in 0..UNRESPONSIVE_AFTER {
            tracker.observe(api::Health::Unreachable, Duration::ZERO, url, 0);
        }

        let report = tracker.observe(api::Health::Serving, Duration::ZERO, url, 0);
        assert_eq!(report.status, Some((ServerState::Serving, Phase::None)));
        assert!(report.log.is_some(), "recovery was never logged");
    }

    /// A healthy server must not generate an event per probe — that would
    /// redraw the whole UI every few seconds for as long as it runs.
    #[test]
    fn a_steady_server_reports_nothing_after_the_first_success() {
        let mut tracker = HealthTracker::default();
        let url = "http://127.0.0.1:1234";

        tracker.observe(api::Health::Serving, Duration::ZERO, url, 0);
        for _ in 0..5 {
            assert_eq!(
                tracker.observe(api::Health::Serving, Duration::ZERO, url, 0),
                Report::default()
            );
        }
    }

    /// STARTING is where a user is most likely to think the thing has hung,
    /// so the two halves of it have to be distinguishable: nothing bound
    /// yet, versus bound and loading weights.
    #[test]
    fn starting_distinguishes_binding_from_loading() {
        let mut tracker = HealthTracker::default();
        let url = "http://127.0.0.1:1234";

        assert_eq!(
            tracker
                .observe(api::Health::Unreachable, Duration::ZERO, url, 0)
                .status,
            Some((ServerState::Starting, Phase::Binding))
        );
        // Still binding: don't re-announce what is already on screen.
        assert_eq!(
            tracker
                .observe(api::Health::Unreachable, Duration::ZERO, url, 0)
                .status,
            None
        );
        assert_eq!(
            tracker
                .observe(api::Health::Loading, Duration::ZERO, url, 0)
                .status,
            Some((ServerState::Starting, Phase::Loading))
        );
        assert_eq!(
            tracker
                .observe(api::Health::Loading, Duration::ZERO, url, 0)
                .status,
            None
        );
    }

    /// The bug this exists to prevent: llama-server downloads its own
    /// weights when a launch finds them missing, and a 16 GiB fetch takes
    /// far longer than the bind budget. Treating a silent port as a failed
    /// bind declared a perfectly healthy download dead after ninety
    /// seconds — which is exactly what it looks like from the outside when
    /// a download "fails silently".
    #[test]
    fn a_download_in_flight_is_not_a_failure_to_bind() {
        let mut tracker = HealthTracker::default();
        let url = "http://127.0.0.1:1234";

        // Bytes landing while the port stays silent.
        let report = tracker.observe(api::Health::Unreachable, Duration::ZERO, url, 1_000_000);
        assert_eq!(
            report.status,
            Some((ServerState::Starting, Phase::Downloading(1_000_000)))
        );

        // Well past the bind budget, still downloading, still not an error.
        let report = tracker.observe(api::Health::Unreachable, BIND_BUDGET * 20, url, 9_000_000);
        assert!(!report.done, "a download was called a failed bind");
        assert_eq!(
            report.status,
            Some((ServerState::Starting, Phase::Downloading(9_000_000)))
        );
    }

    /// A killed download leaves its partial behind, and this cache holds
    /// several. Counting stale bytes as progress would report a download
    /// for a launch that is fetching nothing, and hold off the bind budget
    /// for ten minutes on the strength of bytes that arrived days ago.
    /// Only growth past the launch baseline counts, which the poller
    /// subtracts before the tracker ever sees it.
    #[test]
    fn bytes_that_predate_the_launch_are_not_progress() {
        let mut tracker = HealthTracker::default();
        let url = "http://127.0.0.1:1234";

        // The poller passes 0 because nothing has grown past the baseline.
        tracker.observe(api::Health::Unreachable, Duration::ZERO, url, 0);
        let report = tracker.observe(api::Health::Unreachable, BIND_BUDGET, url, 0);

        assert!(report.done, "a stale partial held off the bind budget");
        assert!(matches!(report.status, Some((ServerState::Error(_), _))));
    }

    /// ...but a download that has stopped dead still fails, measured from
    /// the last byte rather than from the launch.
    #[test]
    fn a_stalled_download_still_times_out() {
        let mut tracker = HealthTracker::default();
        let url = "http://127.0.0.1:1234";

        tracker.observe(api::Health::Unreachable, Duration::ZERO, url, 1_000);
        let report = tracker.observe(api::Health::Unreachable, HEALTH_BUDGET * 2, url, 1_000);

        assert!(report.done, "a dead download waited forever");
        assert!(matches!(report.status, Some((ServerState::Error(_), _))));
    }

    /// A process that never opens its port fails on the short budget. It is
    /// not loading slowly — it never got that far — and making the user
    /// wait out the ten-minute load budget for it teaches them that
    /// STARTING means nothing.
    #[test]
    fn never_binding_the_port_fails_fast() {
        let mut tracker = HealthTracker::default();
        let url = "http://127.0.0.1:1234";

        tracker.observe(api::Health::Unreachable, Duration::ZERO, url, 0);
        let report = tracker.observe(api::Health::Unreachable, BIND_BUDGET, url, 0);

        assert!(report.done, "kept polling past the bind budget");
        assert!(
            matches!(report.status, Some((ServerState::Error(_), _))),
            "got {:?}",
            report.status
        );
        assert!(
            BIND_BUDGET < HEALTH_BUDGET,
            "the bind leash must be shorter"
        );
    }

    /// A slow load must *not* be cut off by the bind budget: once the port
    /// answers 503 the server is plainly working, and a 31B cold load can
    /// legitimately take minutes.
    #[test]
    fn a_slow_load_is_not_cut_off_by_the_bind_budget() {
        let mut tracker = HealthTracker::default();
        let url = "http://127.0.0.1:1234";

        tracker.observe(api::Health::Loading, Duration::ZERO, url, 0);
        let report = tracker.observe(api::Health::Loading, BIND_BUDGET * 3, url, 0);

        assert!(!report.done, "a loading server was killed off by the leash");
    }

    /// Stopping must be bounded even when the process refuses to go
    /// quietly. `stop()` sits on the critical path of every hot-swap, the
    /// `:stop` command and quitting the app, so an unbounded wait here is
    /// felt as the whole UI hanging.
    #[tokio::test]
    async fn a_process_that_ignores_sigterm_is_killed_within_the_grace() {
        let supervisor = Supervisor::new();
        let (tx, _rx) = mpsc::unbounded_channel();

        supervisor
            .spawn_program(
                "/bin/sh",
                LauncherMode::Manual,
                Some("stubborn".into()),
                // The loop matters: with a single trailing command the
                // shell execs it, and an ignored signal survives the exec
                // only by accident. Looping keeps the shell in charge.
                vec![
                    "-c".into(),
                    "trap '' TERM; while :; do sleep 0.2; done".into(),
                ],
                "http://127.0.0.1:1".into(),
                tx,
                None,
            )
            .await
            .expect("spawn /bin/sh");

        // Let the shell actually install the trap. Signalling it before
        // that just kills a shell with default dispositions, which proves
        // nothing about the escalation.
        tokio::time::sleep(Duration::from_millis(500)).await;

        let outcome = tokio::time::timeout(TERM_GRACE + KILL_GRACE * 2, supervisor.stop()).await;

        assert_eq!(
            outcome,
            Ok(Stopped::Killed),
            "a process that ignores SIGTERM must be escalated, not waited on"
        );
        assert!(!supervisor.is_running().await);
    }

    /// A cooperative process gets to exit on SIGTERM, which is what lets
    /// llama-server release its GPU allocation instead of leaving it to the
    /// kernel.
    #[tokio::test]
    async fn a_cooperative_process_is_terminated_not_killed() {
        let supervisor = Supervisor::new();
        let (tx, _rx) = mpsc::unbounded_channel();

        supervisor
            .spawn_program(
                "/bin/sleep",
                LauncherMode::Manual,
                Some("polite".into()),
                vec!["30".into()],
                "http://127.0.0.1:1".into(),
                tx,
                None,
            )
            .await
            .expect("spawn /bin/sleep");

        assert_eq!(supervisor.stop().await, Stopped::Terminated);
    }

    /// Every outcome has to read differently: "nothing was running" and
    /// "it is still shutting down" are opposite situations.
    #[test]
    fn every_stop_outcome_says_something_different() {
        let labels = [
            Stopped::Nothing.label(),
            Stopped::Terminated.label(),
            Stopped::Killed.label(),
            Stopped::Abandoned.label(),
        ];
        for (i, a) in labels.iter().enumerate() {
            for b in labels.iter().skip(i + 1) {
                assert_ne!(a, b);
            }
        }
    }

    /// The most common way a launch fails on a memory-tight machine is the
    /// kernel reclaiming the process. "exited with signal: 9" is a true but
    /// useless thing to tell someone; the diagnosis has to name the cause.
    #[cfg(unix)]
    #[test]
    fn an_oom_kill_is_diagnosed_rather_than_reported_raw() {
        use std::os::unix::process::ExitStatusExt;

        let state = diagnose(ExitStatus::from_raw(libc::SIGKILL));
        let ServerState::Error(reason) = state else {
            panic!("a signal death is not a clean exit");
        };
        assert!(
            reason.contains("memory"),
            "no mention of the likely cause: {reason}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_clean_exit_is_still_off_and_a_failure_is_still_an_error() {
        use std::os::unix::process::ExitStatusExt;

        assert_eq!(diagnose(ExitStatus::from_raw(0)), ServerState::Off);
        // Wait status: exit code in the high byte, no signal in the low.
        assert!(matches!(
            diagnose(ExitStatus::from_raw(1 << 8)),
            ServerState::Error(_)
        ));
    }

    #[tokio::test]
    async fn spawn_missing_binary_reports_error_without_panicking() {
        let supervisor = Supervisor::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let result = supervisor
            .spawn(
                LauncherMode::Manual,
                Some("does-not-matter".into()),
                vec!["--this-flag-does-not-exist".into()],
                "http://127.0.0.1:1".into(),
                tx,
                None,
            )
            .await;
        // BINARY ("llama-server") may or may not be on PATH; either way
        // this must never panic, only return a Result.
        let _ = result;
    }
}
