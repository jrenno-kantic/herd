//! llama-server launcher/supervisor.
//!
//! Two complementary modes, matching the two workflows found in the
//! original shell/JS tooling:
//!
//! - **Router** (`router` command) — spawns a single long-running
//!   `llama-server --models-preset models.ini --models-max N
//!   --sleep-idle-seconds S` process (mirrors `startrouter.sh`). The
//!   router loads/unloads models itself on demand; herd only
//!   supervises the one process and surfaces its logs/status.
//! - **Manual** (`launch <model>` command) — builds the flags for a
//!   *single* model preset from `models.ini` (mirrors `llama-launch.js`)
//!   and spawns `llama-server` directly with that one model. Selecting a
//!   different model stops the previous process first (hot-swap).

pub mod api;
pub mod caps;
pub mod hub;
pub mod ini;
pub mod memory;
pub mod overrides;
pub mod process;
pub mod session;

pub use ini::{default_config_path, load, resolve_config_path, tiers, Tier};
pub use memory::{Budget, Fit};
pub use overrides::Overrides;
pub use process::Supervisor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LauncherMode {
    #[default]
    Idle,
    Router,
    Manual,
}

impl LauncherMode {
    pub fn label(self) -> &'static str {
        match self {
            LauncherMode::Idle => "idle",
            LauncherMode::Router => "router",
            LauncherMode::Manual => "manual",
        }
    }
}

/// Lifecycle of the supervised llama-server process.
///
/// ```text
/// Off ──launch──> Starting ──/health 200──> Serving ──stop──> Stopping ──> Off
///                     │                        │
///                     └─── spawn/load fails ───┴── crash ──> Error ──> Off
/// ```
///
/// `Error` is not one of the four "happy" states but it must exist: a model
/// that OOMs or a missing GGUF would otherwise be indistinguishable from
/// `Off`, and relaunching would hit the same wall silently.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ServerState {
    #[default]
    Off,
    Starting,
    Serving,
    Stopping,
    Error(String),
}

impl ServerState {
    /// Short uppercase tag for the status bar.
    pub fn tag(&self) -> &'static str {
        match self {
            ServerState::Off => "OFF",
            ServerState::Starting => "STARTING",
            ServerState::Serving => "SERVING",
            ServerState::Stopping => "STOPPING",
            ServerState::Error(_) => "ERROR",
        }
    }

    pub fn label(&self) -> String {
        match self {
            ServerState::Error(reason) => format!("ERROR ({reason})"),
            other => other.tag().to_string(),
        }
    }

    /// True while a process is supervised, i.e. a `stop` would do something.
    pub fn is_live(&self) -> bool {
        matches!(
            self,
            ServerState::Starting | ServerState::Serving | ServerState::Stopping
        )
    }
}

/// Sub-state within [`ServerState`], carrying the detail that turns "it
/// seems stuck" into something a user can act on.
///
/// It is deliberately *not* another `ServerState` variant: `is_live()`
/// answers "would a stop do something", and every phase here belongs to a
/// state where the answer is already correct. Splitting them keeps the
/// lifecycle machine four-valued while still letting the status bar say
/// which minute of a five-minute load we are in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    None,
    /// STARTING: the process is alive but nothing answers on the port yet.
    Binding,
    /// STARTING: llama-server is fetching the weights before it can load
    /// them. Carries the bytes on disk so far.
    ///
    /// Its own phase because it is the one kind of "nothing is listening"
    /// that is entirely healthy and can legitimately last half an hour —
    /// treating it as a failed bind declared a 16 GiB download dead after
    /// ninety seconds.
    Downloading(u64),
    /// STARTING: the port answers (503) — weights are loading.
    Loading,
    /// SERVING: `/health` has stopped answering while the process is still
    /// alive. The usual cause on a memory-tight machine is the model being
    /// paged out from under the server, which kills no process and so
    /// would otherwise leave the UI reporting SERVING indefinitely.
    Unresponsive(u32),
}

impl Phase {
    /// Short annotation for the status bar, or `None` when the state alone
    /// says everything.
    pub fn label(self) -> Option<String> {
        match self {
            Phase::None => None,
            Phase::Binding => Some("binding port".into()),
            Phase::Downloading(bytes) => Some(format!("downloading {}", hub::human_bytes(bytes))),
            Phase::Loading => Some("loading weights".into()),
            Phase::Unresponsive(probes) => Some(format!("not responding ({probes} probes missed)")),
        }
    }

    /// True when the phase is itself the bad news, so the UI can colour it
    /// as a warning even though the state is nominally healthy.
    pub fn is_degraded(self) -> bool {
        matches!(self, Phase::Unresponsive(_))
    }
}

/// Snapshot broadcast to the UI whenever the supervised process changes
/// state (spawned, became ready, exited, failed to start...).
#[derive(Debug, Clone)]
pub struct LlamaSnapshot {
    pub state: ServerState,
    pub mode: LauncherMode,
    pub model: Option<String>,
    pub phase: Phase,
}

impl LlamaSnapshot {
    pub fn new(state: ServerState, mode: LauncherMode, model: Option<String>) -> Self {
        Self {
            state,
            mode,
            model,
            phase: Phase::None,
        }
    }

    pub fn with_phase(mut self, phase: Phase) -> Self {
        self.phase = phase;
        self
    }

    pub fn off() -> Self {
        Self::new(ServerState::Off, LauncherMode::Idle, None)
    }
}
