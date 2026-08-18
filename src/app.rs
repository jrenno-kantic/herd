use crate::event::UiEvent;
use crate::services::clipboard;
use crate::services::llama::{
    self,
    api::ChatOutcome,
    hub::Availability,
    ini::LlamaConfig,
    memory,
    overrides::Scope,
    prefs::{self, Prefs, RouterPrefs},
    Budget, Fit, LauncherMode, Overrides, Phase, ServerState, Tier,
};
use crate::services::preflight::Tools;
use chrono::{DateTime, Local};
use crossterm::event::{KeyCode, KeyEvent};
use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

mod screen_input;

const MAX_LOGS: usize = 500;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Screen {
    Models,
    /// What is actually in llama.cpp's model cache, as against what the
    /// active tier names: Models lists what this tier can launch, Hub
    /// lists what this machine has.
    ///
    /// **Last in the menu**, though it answers the other half of the
    /// Models screen's question. The six screens before it are the ones a
    /// session moves through — browse, launch, watch, probe — and Hub is
    /// where you go occasionally, to see what the cache is holding. Menu
    /// order follows how often a screen is wanted, not how related it is
    /// to its neighbour.
    Hub,
    Server,
    /// llama-server's built-in multi-model mode. Sits next to Server
    /// because it is the same lifecycle seen from the other end — one
    /// process that loads and unloads presets by itself — and the digits
    /// are worth renumbering to keep those two together.
    Router,
    Test,
    Stats,
    Settings,
    Logs,
}

impl Screen {
    /// The order of the sidebar menu, and so of the digit shortcuts.
    /// Independent of the enum's own order, which is why moving a screen
    /// here is a one-line change.
    pub const ALL: [Screen; 8] = [
        Screen::Models,
        Screen::Server,
        Screen::Router,
        Screen::Test,
        Screen::Stats,
        Screen::Settings,
        Screen::Logs,
        Screen::Hub,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Screen::Models => "Models",
            Screen::Hub => "Hub",
            Screen::Server => "Server",
            Screen::Router => "Router",
            Screen::Test => "Test",
            Screen::Stats => "Stats",
            Screen::Settings => "Settings",
            Screen::Logs => "Logs",
        }
    }

    fn index(self) -> usize {
        Screen::ALL.iter().position(|&s| s == self).unwrap_or(0)
    }

    pub fn next(self) -> Screen {
        Screen::ALL[(self.index() + 1) % Screen::ALL.len()]
    }

    pub fn prev(self) -> Screen {
        let len = Screen::ALL.len();
        Screen::ALL[(self.index() + len - 1) % len]
    }
}

/// What keystrokes currently mean. `Browse` is the default: letters are
/// shortcuts. Every other mode captures text until `Enter` or `Esc`.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Mode {
    Browse,
    Command,
    Filter,
    EditSetting,
    /// Typing the message to send to the model on the Test screen.
    EditPrompt,
    /// Choosing which `models.ini` to use, from the ones on this machine.
    Picker,
    /// Something is already listening on the configured port and herd
    /// did not start it. Waiting for the user to confirm or cancel.
    ConfirmLaunch,
    /// A supervised server is live and normal quit waits for confirmation.
    ConfirmQuit,
    /// The `?` overlay. A mode rather than a screen so it can be summoned
    /// from anywhere and dismissed back to where the user was.
    Help,
    /// The `:help` overlay — the same idea for the command bar's
    /// vocabulary. Separate from `Help` because the two answer different
    /// questions ("what does this key do" against "what can I type"), and
    /// one list of both would bury each in the other.
    Commands,
    /// The `:about` overlay: which build this is, and what it is running
    /// against. A third reference card beside `Help` and `Commands`,
    /// answering the third question a stuck user has — "what am I
    /// running?" — which is otherwise a tour of four screens.
    About,
    /// About to delete a cached model. Its own mode rather than another
    /// `Confirm` variant: every other prompt asks whether to *start*
    /// something, and answering `y` to the wrong one of those costs a
    /// launch. This one costs the download.
    ConfirmDelete,
}

#[derive(Debug, Clone)]
pub enum Action {
    None,
    Quit,
    RunCommand(String),
    /// The user switched tier. `main.rs` forwards this to the `Executor`
    /// so the next launch resolves against the newly selected file rather
    /// than the one picked at startup.
    ConfigPathChanged(PathBuf),
    /// Send a chat probe to the running model (the `test_call.sh`
    /// equivalent). Carried as structured data rather than a command
    /// string, because the prompt is free text and must not be re-parsed.
    RunChat {
        model: String,
        prompt: String,
    },
    /// Put a line of text on the system clipboard. Structured for the
    /// same reason as `RunChat`: the payload is a shell command, complete
    /// with quotes and spaces, and must never be re-parsed out of a
    /// command string.
    CopyToClipboard {
        /// What was copied, for the log line the Executor writes once the
        /// clipboard has actually taken it.
        label: String,
        text: String,
    },
    /// Remove a cached model from the hub cache. Carries the repo rather
    /// than a path: the path is computed inside `hub::delete_repo`, which
    /// is where the fence around it lives, and a path travelling through
    /// the UI is a path something could rewrite on the way.
    DeleteModel {
        reference: String,
        repo: String,
    },
    /// Fetch a preset's artifacts, optionally launching it afterwards.
    /// Like `RunChat`, structured rather than a command string: the repo
    /// reference and the "and then launch" flag must not be re-parsed out
    /// of a line of text.
    Download {
        model: String,
        repo: String,
        wants: llama::hub::Wants,
        then_launch: bool,
    },
}

/// One row of the Models table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRow {
    pub name: String,
    pub repo: String,
    pub ctx: String,
    pub spec: String,
}

/// One row of the Hub screen: a model llama.cpp has in its cache, whether
/// the active tier has a preset for it, and what it costs on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubRow {
    /// `repo:quant`, as `--cache-list` spells it.
    pub reference: String,
    /// The weights of this quantisation in the current revision.
    pub weights: Option<u64>,
    /// Everything the repo occupies, stale revisions and projectors
    /// included.
    pub disk: Option<u64>,
    /// The preset in the active tier that names this repo, if any.
    /// `None` is what the screen colours: a model taking up disk that
    /// nothing in this tier can launch.
    pub preset: Option<String>,
    /// Another cached entry shares this repo's directory, so `disk` is not
    /// this model's alone. Said rather than divided: the cache keeps no
    /// per-quantisation accounting, and splitting it would be inventing a
    /// number nobody measured.
    pub shares_disk: bool,
}

/// One line of the Settings screen. Headers are rendered but not
/// selectable; the cursor only ever lands on `Entry` rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingRow {
    Header(String),
    Entry {
        scope: Scope,
        /// Preset the entry belongs to, empty for global rows.
        model: String,
        key: String,
        ini_value: String,
        override_value: Option<String>,
    },
}

impl SettingRow {
    pub fn as_entry(&self) -> Option<(&Scope, &str, &str, &str, Option<&str>)> {
        match self {
            SettingRow::Header(_) => None,
            SettingRow::Entry {
                scope,
                model,
                key,
                ini_value,
                override_value,
            } => Some((
                scope,
                model.as_str(),
                key.as_str(),
                ini_value.as_str(),
                override_value.as_deref(),
            )),
        }
    }
}

/// One editable number on the Router screen.
///
/// The two settings the router mode is actually about — how many models
/// stay loaded, and how long an idle one survives — carried as data so the
/// screen renders them from a list rather than from two hand-written
/// lines, and so `+`/`-` has something to index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouterSetting {
    pub key: &'static str,
    pub value: u32,
    pub unit: &'static str,
    pub describes: &'static str,
}

impl RouterSetting {
    pub fn value_label(&self) -> String {
        format!("{}{}", self.value, self.unit)
    }
}

/// Counters for the current serving session, reset on every launch so
/// they describe *this* model rather than the whole herd run.
#[derive(Debug, Clone, Default)]
pub struct SessionStats {
    /// Wall-clock launch time. Kept alongside the monotonic `started_at`
    /// because an `Instant` can express "12m ago" but never "started at
    /// 14:32", which is what a stats page is asked for.
    pub started_at: Option<DateTime<Local>>,
    pub probes: usize,
    pub failures: usize,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_latency: std::time::Duration,
    pub last_rate: Option<f64>,
    pub best_rate: Option<f64>,
    /// Time to first token from **the first probe after the model loaded**
    /// — the cold one, and the headline figure.
    ///
    /// Only that request measures a cold model: it finds the weights not
    /// yet resident and the cache empty. It is kept apart from the running
    /// figures below rather than averaged into them, because a mean over
    /// both drifts towards the warm value the more probes are run and
    /// describes neither.
    ///
    /// `None` until the first probe answers, and `None` afterwards if that
    /// probe's server sent no `timings` — the second probe is not a
    /// stand-in for it.
    pub first_token: Option<std::time::Duration>,
    /// The most recent probe's time to first token, and the mean over
    /// every probe that reported one.
    ///
    /// These describe the **warm** model, which is the other half of the
    /// question: `first_token` says how long until it was usable at all,
    /// and these say what it costs per request once it is. Counted over
    /// their own probe count rather than `probes`, because a server that
    /// sends no `timings` still answers and averaging it in as a zero
    /// would quietly halve the figure.
    pub last_ttft: Option<std::time::Duration>,
    pub ttft_probes: usize,
    pub total_ttft: std::time::Duration,
}

impl SessionStats {
    fn begin(&mut self) {
        *self = Self {
            started_at: Some(Local::now()),
            ..Self::default()
        };
    }

    fn record(&mut self, outcome: &ChatOutcome) {
        // Read before the counter moves: this is what makes it the *first*
        // probe rather than the most recent one.
        let is_first = self.probes == 0;
        self.probes += 1;
        self.prompt_tokens += outcome.prompt_tokens.unwrap_or(0);
        self.completion_tokens += outcome.completion_tokens.unwrap_or(0);
        self.total_latency += outcome.latency;
        self.last_rate = outcome.tokens_per_second;

        if let Some(rate) = outcome.tokens_per_second {
            self.best_rate = Some(self.best_rate.map_or(rate, |best| best.max(rate)));
        }

        if let Some(ttft) = outcome.ttft() {
            self.last_ttft = Some(ttft);
            self.ttft_probes += 1;
            self.total_ttft += ttft;
        }

        // The cold-start measurement, taken once. `SessionStats` is reset
        // on every `Starting`, so "the first probe of the session" and
        // "the first call after this model loaded" are the same thing.
        if is_first {
            self.first_token = outcome.ttft();
        }
    }

    /// Mean time to first token over the probes that reported one.
    pub fn average_ttft(&self) -> Option<std::time::Duration> {
        (self.ttft_probes > 0).then(|| self.total_ttft / self.ttft_probes as u32)
    }

    /// Output tokens per second across every probe of this session, which
    /// smooths out the one-off cost of the first request.
    pub fn average_rate(&self) -> Option<f64> {
        let seconds = self.total_latency.as_secs_f64();
        (self.completion_tokens > 0 && seconds > 0.0)
            .then(|| self.completion_tokens as f64 / seconds)
    }

    pub fn started_label(&self) -> String {
        self.started_at
            .map(|at| at.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| "-".into())
    }
}

/// One selectable `models.ini` on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigChoice {
    pub label: String,
    pub path: PathBuf,
    pub presets: usize,
    /// Presets in this file that the current budget cannot hold.
    pub too_large: usize,
}

/// Why the launcher is asking before it launches.
///
/// Both cases are "this is probably not what you want, but it is your
/// machine": herd states the problem and lets the user decide, rather
/// than refusing or silently proceeding.
#[derive(Debug, Clone, PartialEq)]
pub enum Confirm {
    /// Something herd did not start already holds the port. Carries the
    /// force command a yes re-dispatches (`launch! …`, `router! …`),
    /// authored by the Executor that refused it — see
    /// [`UiEvent::PortInUse`].
    PortInUse { port: u16, retry: String },
    /// The preset is estimated to need more memory than the budget allows.
    /// Worth interrupting for: on a machine with no headroom this is the
    /// difference between a launch that fails and a swap storm that takes
    /// the whole desktop down with it.
    TooLarge { estimate: f64, budget: f64 },
    /// The weights are not on this machine. Launching would fetch them,
    /// which on a domestic connection is tens of minutes and several
    /// gigabytes — not something to start without being asked.
    NotDownloaded { repo: String },
}

/// A cached model the user has asked to delete, held while the prompt is
/// up.
///
/// Carries what the prompt has to say out loud, resolved *before* the
/// question is asked rather than after it is answered: how much goes, and
/// how many other quantisations share the directory and would go with it.
/// A prompt that says "delete this model?" and silently takes a second one
/// with it has not asked the question the user answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDelete {
    pub reference: String,
    pub repo: String,
    pub bytes: Option<u64>,
    /// Other cached quantisations in the same repo directory.
    pub also_removes: Vec<String>,
}

/// A download in flight, in bytes rather than percent so the bar and the
/// "2.1G of 6.7G" beside it cannot disagree.
#[derive(Debug, Clone, PartialEq)]
pub struct Download {
    pub model: String,
    pub done: u64,
    pub total: u64,
}

impl Download {
    pub fn ratio(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.done as f64 / self.total as f64).clamp(0.0, 1.0)
    }

    /// `2.1G of 6.7G · 31%`
    pub fn label(&self) -> String {
        format!(
            "{} of {} · {:.0}%",
            llama::hub::human_bytes(self.done),
            llama::hub::human_bytes(self.total),
            self.ratio() * 100.0
        )
    }
}

/// Live view of the supervised process.
#[derive(Debug, Clone, Default)]
pub struct ServerRuntime {
    pub state: ServerState,
    pub mode: LauncherMode,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    /// Detail within the state: which part of startup, or that a serving
    /// process has gone quiet.
    pub phase: Phase,
    started_at: Option<Instant>,
}

impl ServerRuntime {
    pub fn uptime_secs(&self) -> Option<u64> {
        self.started_at.map(|start| start.elapsed().as_secs())
    }

    /// Is the server nominally up but not actually answering? Rendered as
    /// a warning rather than a healthy SERVING.
    pub fn is_degraded(&self) -> bool {
        self.phase.is_degraded()
    }

    /// `mm:ss` since the current state was entered, for the states where
    /// "how long has this been going on" is the user's actual question.
    /// Driven off the existing tick, so it updates without any new events.
    pub fn elapsed_label(&self) -> Option<String> {
        let total = self.uptime_secs()?;
        Some(format!("{:02}:{:02}", total / 60, total % 60))
    }
}

/// Everything the launcher UI needs: the parsed `models.ini`, the tiers
/// available on this machine, the browse cursor and filter, the session
/// overrides, and the live server state.
#[derive(Debug)]
pub struct LauncherState {
    pub config_path: PathBuf,
    pub config: Option<LlamaConfig>,
    pub config_error: Option<String>,
    pub tiers: Vec<Tier>,
    pub ram_gib: Option<u64>,
    pub cursor: usize,
    pub filter: String,
    pub overrides: Overrides,
    /// Presets marked with a star, by name. Kept across restarts in
    /// `~/.herd_config`, and keyed by name rather than by tier because
    /// `gemma4-12b` in the 16gb tier and in the 32gb one are the same
    /// model — see `prefs.rs`.
    pub favorites: BTreeSet<String>,
    /// Presets with the `[mono-focus]` profile switched on, by name.
    /// Persisted alongside the favourites and for the same reason: it is
    /// something the user chose, not where the program was.
    pub mono_focus: BTreeSet<String>,
    /// What the Router screen would start llama-server's built-in router
    /// with. Persisted, since they are settings, not session state.
    pub router: RouterPrefs,
    /// Which of the two router settings the Router screen has selected.
    pub router_cursor: usize,
    pub settings_cursor: usize,
    pub edit_buffer: String,
    pub server: ServerRuntime,
    /// Model awaiting confirmation, and what the launcher is asking about.
    pub pending_launch: Option<String>,
    pub confirm: Option<Confirm>,
    /// What `llama-server --cache-list` last reported, with the sizes
    /// measured off the cache directory. `None` means we have not managed
    /// to ask, which is why `availability` answers `Unknown` rather than
    /// "missing" — the same restraint as `Fit`.
    pub cached: Option<Vec<llama::hub::CachedModel>>,
    /// Where the Hub screen's cursor is, within [`LauncherState::hub_rows`].
    pub hub_cursor: usize,
    /// The deletion awaiting an answer, if any.
    pub pending_delete: Option<PendingDelete>,
    /// The download in flight, if any.
    pub download: Option<Download>,
    /// Last preset actually launched, remembered across restarts.
    pub last_launched: Option<String>,
    /// Test screen: the message sent to the model, the last outcome, and
    /// whether a probe is in flight.
    pub prompt: String,
    pub chat: Option<Result<ChatOutcome, String>>,
    pub chat_pending: bool,
    /// When the in-flight probe was dispatched, so the screen can count up
    /// while waiting. A generation against a large model can take a long
    /// time, and a motionless "waiting for the model…" is exactly the kind
    /// of silence that reads as a hang.
    chat_started: Option<Instant>,
    pub stats: SessionStats,
    /// Share of memory held back for the OS. Session-only, like the other
    /// overrides.
    pub reserved_ratio: f64,
    pub picker_cursor: usize,
    /// Lines of the argv preview hidden *above* its viewport. Zero is the
    /// top, which is the resting position — unlike the Logs screen, where
    /// the interesting end is the bottom.
    pub preview_scroll: usize,
}

impl LauncherState {
    fn new(config_path: PathBuf, last_launched: Option<String>, prefs: Prefs) -> Self {
        let mut state = Self {
            config_path,
            config: None,
            config_error: None,
            tiers: llama::tiers(),
            ram_gib: llama::ini::installed_ram_gib(),
            cursor: 0,
            filter: String::new(),
            overrides: prefs.overrides,
            favorites: prefs.favorites,
            mono_focus: prefs.mono_focus,
            router: prefs.router,
            router_cursor: 0,
            settings_cursor: 0,
            edit_buffer: String::new(),
            server: ServerRuntime::default(),
            pending_launch: None,
            confirm: None,
            cached: None,
            hub_cursor: 0,
            pending_delete: None,
            download: None,
            last_launched,
            prompt: llama::api::DEFAULT_PROMPT.to_string(),
            chat: None,
            chat_pending: false,
            chat_started: None,
            stats: SessionStats::default(),
            reserved_ratio: memory::DEFAULT_RESERVED_RATIO,
            picker_cursor: 0,
            preview_scroll: 0,
        };
        state.reload();
        state.restore_cursor();
        state
    }

    /// Re-reads `models.ini` from disk. Cheap (local file, no async
    /// needed) so it can run synchronously on the UI thread.
    pub fn reload(&mut self) {
        match llama::load(&self.config_path) {
            Ok(config) => {
                self.config = Some(config);
                self.config_error = None;
            }
            Err(error) => {
                self.config = None;
                self.config_error = Some(error);
            }
        }
        self.clamp_cursor();
        self.clamp_settings_cursor();
    }

    /// Puts the cursor back on the remembered preset if it exists in the
    /// current tier — the common case is relaunching the same model.
    fn restore_cursor(&mut self) {
        let Some(last) = self.last_launched.clone() else {
            return;
        };
        if let Some(index) = self.rows().iter().position(|row| row.name == last) {
            self.cursor = index;
        }
    }

    pub fn is_favorite(&self, model: &str) -> bool {
        self.favorites.contains(model)
    }

    /// Stars the highlighted preset, or takes the star off. Returns what
    /// it did, so the caller can say so without asking again.
    ///
    /// The list is deliberately **not** reordered around favourites. A
    /// table people navigate by position — third row down, the one under
    /// the 12B — must not rearrange itself because a star was added
    /// somewhere above; the star is a marker on the row you already know,
    /// not a new sort order.
    fn toggle_favorite(&mut self) -> Option<(String, bool)> {
        let model = self.selected_model()?;

        let starred = if self.favorites.remove(&model) {
            false
        } else {
            self.favorites.insert(model.clone());
            true
        };
        Some((model, starred))
    }

    /// The two router numbers as editable rows, in the order the screen
    /// shows them.
    pub fn router_rows(&self) -> [RouterSetting; 2] {
        [
            RouterSetting {
                key: "models-max",
                value: self.router.models_max,
                unit: "",
                describes: "models kept resident before one is unloaded",
            },
            RouterSetting {
                key: "sleep-idle-seconds",
                value: self.router.sleep_idle_seconds,
                unit: "s",
                describes: "idle time before a model is unloaded",
            },
        ]
    }

    /// Steps the highlighted router setting, snapped into range.
    ///
    /// `models-max` moves by one and idle time by half a minute: the
    /// useful idle range is an hour wide, and stepping that a second at a
    /// time is not editing, it is waiting.
    fn adjust_router(&mut self, up: bool) {
        let step = |value: u32, step: u32, (low, high): (u32, u32)| -> u32 {
            if up {
                value.saturating_add(step).min(high)
            } else {
                value.saturating_sub(step).max(low)
            }
        };

        match self.router_cursor {
            0 => {
                self.router.models_max = step(self.router.models_max, 1, prefs::MODELS_MAX_RANGE);
            }
            _ => {
                self.router.sleep_idle_seconds = step(
                    self.router.sleep_idle_seconds,
                    prefs::SLEEP_IDLE_STEP,
                    prefs::SLEEP_IDLE_RANGE,
                );
            }
        }
    }

    /// The argv `enter` on the Router screen would spawn, for the preview.
    pub fn router_argv_preview(&self) -> Result<Vec<String>, String> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| "no config loaded".to_string())?;

        Ok(llama::ini::build_router_args(
            config,
            self.router.models_max,
            self.router.sleep_idle_seconds,
            &[],
        ))
    }

    /// The same, as one line a shell will run.
    pub fn router_shell_command(&self) -> Result<String, String> {
        self.router_argv_preview()
            .map(|argv| clipboard::shell_command(llama::process::BINARY, &argv))
    }

    pub fn model_names(&self) -> Vec<String> {
        self.config
            .as_ref()
            .map(|config| {
                config
                    .model_names()
                    .into_iter()
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Table rows for the Models screen, honouring the active filter.
    pub fn rows(&self) -> Vec<ModelRow> {
        let Some(config) = self.config.as_ref() else {
            return Vec::new();
        };

        config
            .model_names()
            .into_iter()
            .map(|name| ModelRow {
                name: name.to_string(),
                repo: effective(config, name, &["hf-repo", "hf", "model"])
                    .unwrap_or_else(|| "-".into()),
                ctx: effective(config, name, &["ctx-size"]).unwrap_or_else(|| "-".into()),
                spec: effective(config, name, &["spec-type"])
                    .map(|value| value.replace("draft-", ""))
                    .unwrap_or_else(|| "-".into()),
            })
            .filter(|row| self.matches_filter(row))
            .collect()
    }

    fn matches_filter(&self, row: &ModelRow) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        let needle = self.filter.to_lowercase();
        row.name.to_lowercase().contains(&needle) || row.repo.to_lowercase().contains(&needle)
    }

    pub fn selected_model(&self) -> Option<String> {
        self.rows().get(self.cursor).map(|row| row.name.clone())
    }

    fn clamp_cursor(&mut self) {
        let len = self.rows().len();
        if len == 0 {
            self.cursor = 0;
        } else if self.cursor >= len {
            self.cursor = len - 1;
        }
    }

    /// The exact argv that launching the highlighted preset would spawn,
    /// overrides included. This is what `llama-launch.js` prints, shown
    /// continuously instead of on demand.
    pub fn argv_preview(&self) -> Result<Vec<String>, String> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| "no config loaded".to_string())?;
        let model = self
            .selected_model()
            .ok_or_else(|| "no model selected".to_string())?;

        self.launch_settings().argv(config, &model, &[])
    }

    /// Everything a launch needs from this screen's state: the overrides,
    /// and which presets have mono-focus on.
    ///
    /// Handed to the `Executor` before each command so the process it
    /// spawns is the one the preview drew. They used to be assembled
    /// separately and had diverged: the preview applied the overrides and
    /// the launch did not.
    pub fn launch_settings(&self) -> llama::ini::LaunchSettings {
        llama::ini::LaunchSettings {
            overrides: self.overrides.clone(),
            mono_focus: self.mono_focus.clone(),
        }
    }

    /// The same launch, as one line a shell will run.
    ///
    /// Built from `argv_preview` rather than from the wrapped text on
    /// screen, so what is copied is the argv herd would spawn — session
    /// overrides included — and not a re-parse of a rendering.
    pub fn shell_command(&self) -> Result<String, String> {
        self.argv_preview()
            .map(|argv| clipboard::shell_command(llama::process::BINARY, &argv))
    }

    pub fn tier_name(&self) -> Option<&str> {
        self.tiers
            .iter()
            .find(|tier| tier.config_path == self.config_path)
            .map(|tier| tier.name.as_str())
    }

    /// Moves to the next/previous tier and reloads. No-op when the machine
    /// has no tiered layout at all.
    fn cycle_tier(&mut self, forward: bool) {
        if self.tiers.is_empty() {
            return;
        }
        let current = self
            .tiers
            .iter()
            .position(|tier| tier.config_path == self.config_path);

        let len = self.tiers.len();
        let next = match current {
            Some(index) if forward => (index + 1) % len,
            Some(index) => (index + len - 1) % len,
            None => 0,
        };

        self.config_path = self.tiers[next].config_path.clone();
        self.cursor = 0;
        self.settings_cursor = 0;
        self.reload();
        self.restore_cursor();
    }

    /// Flat Settings model: `[server]`, then `[*]`, then the selected
    /// preset's own keys. Effective values come from the ini; an override
    /// is shown alongside rather than replacing it, so the user can always
    /// see what they diverged from.
    pub fn setting_rows(&self) -> Vec<SettingRow> {
        let Some(config) = self.config.as_ref() else {
            return Vec::new();
        };
        let model = self.selected_model().unwrap_or_default();
        let mut rows = Vec::new();

        rows.push(SettingRow::Header("[server]".into()));
        for (key, value) in config.server.iter() {
            rows.push(self.entry(Scope::Global, "", key, value));
        }

        rows.push(SettingRow::Header("[*]  defaults".into()));
        for (key, value) in config.defaults.iter() {
            rows.push(self.entry(Scope::Global, "", key, value));
        }

        if let Some(section) = config.model(&model) {
            rows.push(SettingRow::Header(format!("[{model}]")));
            for (key, value) in section.iter() {
                rows.push(self.entry(Scope::Model, &model, key, value));
            }
        }

        // The profile last, because it is the last thing applied — the
        // section reads in the order the flags actually take effect.
        //
        // Its keys are listed **only when it is on**. Showing them while
        // it is off would put rows on screen that look like every other
        // editable setting and are not in force, and editing one would
        // quietly create a model override that *is* — which is the
        // confusion this ordering avoids rather than explains.
        rows.push(SettingRow::Header(self.mono_focus_header()));
        if self.mono_focus_on(&model) {
            for (key, value) in config.mono_focus.iter() {
                rows.push(self.entry(Scope::Model, &model, key, value));
            }
        }

        rows
    }

    /// The `[mono-focus]` heading, which carries the state because there
    /// is nowhere else to put it: an empty section and a switched-off one
    /// look identical otherwise.
    fn mono_focus_header(&self) -> String {
        let Some(config) = self.config.as_ref() else {
            return format!("[{}]", llama::ini::MONO_FOCUS);
        };
        let name = llama::ini::MONO_FOCUS;

        if config.mono_focus.iter().next().is_none() {
            return format!("[{name}]  not in this models.ini");
        }

        match self.selected_model() {
            Some(model) if self.mono_focus_on(&model) => format!("[{name}]  ON  ·  m to disable"),
            _ => format!("[{name}]  off  ·  m to enable"),
        }
    }

    pub fn mono_focus_on(&self, model: &str) -> bool {
        self.mono_focus.contains(model)
    }

    /// Switches the profile for the highlighted preset. Returns what it
    /// did, or `None` when there is nothing to switch — an ini with no
    /// `[mono-focus]` section has no profile to apply, and pretending
    /// otherwise would set a flag that changes nothing.
    fn toggle_mono_focus(&mut self) -> Option<(String, bool)> {
        let model = self.selected_model()?;

        if self
            .config
            .as_ref()
            .is_none_or(|config| config.mono_focus.iter().next().is_none())
        {
            return None;
        }

        let on = if self.mono_focus.remove(&model) {
            false
        } else {
            self.mono_focus.insert(model.clone());
            true
        };
        Some((model, on))
    }

    fn entry(&self, scope: Scope, model: &str, key: &str, ini_value: &str) -> SettingRow {
        SettingRow::Entry {
            scope,
            model: model.to_string(),
            key: key.to_string(),
            ini_value: ini_value.to_string(),
            override_value: self.overrides.get(scope, model, key).map(str::to_string),
        }
    }

    /// Indices of selectable rows within `setting_rows()`.
    pub fn setting_entry_indices(&self) -> Vec<usize> {
        self.setting_rows()
            .iter()
            .enumerate()
            .filter(|(_, row)| row.as_entry().is_some())
            .map(|(index, _)| index)
            .collect()
    }

    pub fn selected_setting(&self) -> Option<SettingRow> {
        let rows = self.setting_rows();
        let index = *self.setting_entry_indices().get(self.settings_cursor)?;
        rows.get(index).cloned()
    }

    /// Memory the machine can give a model, after the reserved share.
    pub fn budget(&self) -> Budget {
        Budget::new(self.ram_gib.unwrap_or(0) as f64, self.reserved_ratio)
    }

    /// Resident size of a preset in GiB, or `None` when nothing supports a
    /// number: no file on disk *and* no parseable name.
    pub fn estimate_gib(&self, name: &str) -> Option<f64> {
        self.sizing(name).map(|sizing| sizing.gib())
    }

    /// The same number, saying where it came from.
    ///
    /// A preset whose weights are already here is **measured**, not
    /// guessed: the heuristic reads a parameter count and a bit-width out
    /// of the repo name and is documented as approximate, but once the file
    /// exists there is a real size to read and no reason to keep guessing.
    /// The measurement is the gguf the current revision would load, plus
    /// the same runtime allowance the estimate adds, so the two columns
    /// stay comparable row to row.
    ///
    /// No I/O happens here: the sizes were measured when `--cache-list` was
    /// read, and this is a lookup. `estimate_gib` is called once per row per
    /// frame, and a stat call per row per frame is not free on a machine
    /// already paging.
    pub fn sizing(&self, name: &str) -> Option<memory::Sizing> {
        let repo = self.repo_of(name)?;

        let measured = self
            .cached
            .as_ref()
            .and_then(|cached| llama::hub::measured_weights(&repo, cached))
            .map(memory::measured_gib);

        match measured {
            Some(gib) => Some(memory::Sizing::Measured(gib)),
            None => memory::estimate_gib(&repo).map(memory::Sizing::Estimated),
        }
    }

    /// Does anything on screen advance on its own?
    ///
    /// Only the clocks do: the server's elapsed/uptime line and the "waiting
    /// for the model" counter. Everything else — the table, the logs, the
    /// download bar — is redrawn by the event that changed it.
    ///
    /// Used to skip the redraw on an idle tick. At rest that was four full
    /// renders a second forever, for a screen that had not changed.
    pub fn ticking(&self) -> bool {
        self.server.elapsed_label().is_some() || self.chat_pending
    }

    /// How long the in-flight probe has been running, or `None` when none
    /// is. Read every tick, which is what makes the counter move.
    pub fn chat_elapsed(&self) -> Option<std::time::Duration> {
        self.chat_pending
            .then(|| self.chat_started.map(|start| start.elapsed()))
            .flatten()
    }

    /// The `hf-repo` a preset names, if it names one.
    pub fn repo_of(&self, name: &str) -> Option<String> {
        let config = self.config.as_ref()?;
        effective(config, name, &["hf-repo", "hf", "model"])
    }

    /// Everything llama.cpp has in its cache, and what the active tier
    /// makes of it.
    ///
    /// Empty until `--cache-list` has answered, which is the same restraint
    /// `availability` shows: an empty list is what a machine with no models
    /// looks like, and claiming that before asking would be a lie in the
    /// one direction that costs a re-download.
    pub fn hub_rows(&self) -> Vec<HubRow> {
        let Some(cached) = self.cached.as_ref() else {
            return Vec::new();
        };

        // Every preset's repo once, so the lookup below is not quadratic in
        // a tier with a dozen presets and a cache with a dozen models.
        let presets: Vec<(String, String)> = self
            .model_names()
            .into_iter()
            .filter_map(|name| self.repo_of(&name).map(|repo| (repo, name)))
            .collect();

        cached
            .iter()
            .map(|entry| HubRow {
                reference: entry.reference.clone(),
                weights: entry.weights,
                disk: entry.bytes,
                // Matched on the repo, the same rule `availability` uses: a
                // tier that names the repo is using what is in it, whatever
                // tag either side spells it with.
                preset: presets
                    .iter()
                    .find(|(repo, _)| {
                        llama::hub::split_repo(repo)
                            .0
                            .eq_ignore_ascii_case(entry.repo())
                    })
                    .map(|(_, name)| name.clone()),
                shares_disk: cached.iter().filter(|other| other.same_repo(entry)).count() > 1,
            })
            .collect()
    }

    pub fn selected_hub(&self) -> Option<HubRow> {
        self.hub_rows().get(self.hub_cursor).cloned()
    }

    fn clamp_hub_cursor(&mut self) {
        let len = self.hub_rows().len();
        if len == 0 {
            self.hub_cursor = 0;
        } else if self.hub_cursor >= len {
            self.hub_cursor = len - 1;
        }
    }

    /// Disk held by the cache, counting each repo once.
    ///
    /// Once, because two quantisations of one repo report the same
    /// directory total — summing the rows would double it, which is exactly
    /// the mistake the `shares_disk` flag exists to avoid making on screen.
    pub fn hub_disk_bytes(&self) -> u64 {
        let Some(cached) = self.cached.as_ref() else {
            return 0;
        };

        let mut seen: BTreeSet<String> = BTreeSet::new();
        cached
            .iter()
            .filter(|entry| seen.insert(entry.repo().to_ascii_lowercase()))
            .filter_map(|entry| entry.bytes)
            .sum()
    }

    /// Whether the weights are on this machine. `Unknown` until llama.cpp
    /// has been asked — never a guess.
    pub fn availability(&self, name: &str) -> Availability {
        let (Some(cached), Some(repo)) = (self.cached.as_ref(), self.repo_of(name)) else {
            return Availability::Unknown;
        };
        llama::hub::availability(&repo, cached)
    }

    /// Why Enter on the Server screen would be a mistake, if it would.
    ///
    /// Relaunching the preset that is already up is a stop and a full
    /// reload for no gain — on a machine where that reload is minutes, an
    /// accidental one is expensive. The Models screen deliberately still
    /// allows it: pressing Enter there after changing a setting is how a
    /// session override is applied, and that is a real workflow. Here
    /// Enter is a convenience, and the convenient thing is not to bounce
    /// the server by accident.
    ///
    /// Shared with the Server screen's display so the reason shown and the
    /// reason enforced cannot drift apart.
    pub fn relaunch_blocked(&self) -> Option<String> {
        let active = self.server.model.as_deref()?;

        if !self.server.state.is_live() || self.selected_model().as_deref() != Some(active) {
            return None;
        }

        Some(format!(
            "{active} is already {} — relaunch it from the Models screen",
            self.server.state.tag().to_ascii_lowercase()
        ))
    }

    /// Optimisations baked into a preset's weights.
    pub fn optimisations(&self, name: &str) -> Vec<llama::caps::Optimisation> {
        self.repo_of(name)
            .map(|repo| llama::caps::optimisations(&repo))
            .unwrap_or_default()
    }

    /// What a preset can do, and whether it is switched on.
    ///
    /// Both halves come from the preset itself — its repo reference and
    /// its own keys — so a capability is never claimed on the strength of
    /// what a model family is generally known to do.
    pub fn capabilities(&self, name: &str) -> Vec<llama::caps::Trait> {
        let Some(repo) = self.repo_of(name) else {
            return Vec::new();
        };
        let value = |key: &str| {
            self.config
                .as_ref()
                .and_then(|config| effective(config, name, &[key]))
        };

        llama::caps::capabilities(
            &repo,
            value("no-mmproj").is_some_and(|v| is_on(&v)),
            value("spec-type").as_deref(),
        )
    }

    /// Which extra artifacts this preset actually uses.
    ///
    /// Read from the preset rather than assumed: `no-mmproj = true` means
    /// the vision projector is dead weight, and only a `draft-mtp`
    /// speculative type uses the MTP head. Together they are a few hundred
    /// megabytes that would otherwise be fetched for nothing.
    pub fn wants(&self, name: &str) -> llama::hub::Wants {
        let Some(config) = self.config.as_ref() else {
            return llama::hub::Wants::default();
        };
        let value = |key: &str| effective(config, name, &[key]);

        llama::hub::Wants {
            mmproj: !value("no-mmproj").is_some_and(|v| is_on(&v)),
            mtp: value("spec-type").is_some_and(|spec| spec.to_ascii_lowercase().contains("mtp")),
        }
    }

    pub fn fit(&self, name: &str) -> Fit {
        // With no RAM reading there is no budget, so nothing is claimed.
        if self.ram_gib.is_none() {
            return Fit::Unknown;
        }
        self.budget().fit(self.estimate_gib(name))
    }

    /// Every `models.ini` selectable on this machine: the discovered RAM
    /// tiers, plus the active file when it lives outside them (a
    /// `--config` path, or the legacy flat layout).
    pub fn config_choices(&self) -> Vec<ConfigChoice> {
        let mut choices: Vec<ConfigChoice> = self
            .tiers
            .iter()
            .map(|tier| self.describe_config(&tier.name, &tier.config_path))
            .collect();

        if !choices.iter().any(|choice| choice.path == self.config_path) {
            choices.insert(0, self.describe_config("current", &self.config_path));
        }

        choices
    }

    /// Counts the presets of a file and how many exceed the budget. Reads
    /// the file directly: the point of the picker is to judge a config
    /// that is *not* the loaded one.
    fn describe_config(&self, label: &str, path: &std::path::Path) -> ConfigChoice {
        let budget = self.budget();
        let (presets, too_large) = match llama::load(path) {
            Err(_) => (0, 0),
            Ok(config) => {
                let names = config.model_names();
                let over = names
                    .iter()
                    .filter(|name| {
                        let repo = effective(&config, name, &["hf-repo", "hf", "model"]);
                        let estimate = repo.as_deref().and_then(memory::estimate_gib);
                        self.ram_gib.is_some() && budget.fit(estimate) == Fit::TooLarge
                    })
                    .count();
                (names.len(), over)
            }
        };

        ConfigChoice {
            label: label.to_string(),
            path: path.to_path_buf(),
            presets,
            too_large,
        }
    }

    /// Which model a chat probe should address: whatever is actually
    /// loaded, falling back to the highlighted preset so the screen is
    /// still useful against a server started outside herd.
    pub fn test_target(&self) -> Option<String> {
        if self.server.state.is_live() {
            if let Some(model) = self.server.model.clone() {
                return Some(model);
            }
        }
        self.selected_model()
    }

    fn clamp_settings_cursor(&mut self) {
        let len = self.setting_entry_indices().len();
        if len == 0 {
            self.settings_cursor = 0;
        } else if self.settings_cursor >= len {
            self.settings_cursor = len - 1;
        }
    }
}

/// First key present for `model`, walking the ini precedence chain
/// (`[model]` -> `[*]` -> `[server]`).
fn effective(config: &LlamaConfig, model: &str, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = config.model(model).and_then(|section| section.get(key)) {
            return Some(value.to_string());
        }
        if let Some(value) = config.defaults.get(key) {
            return Some(value.to_string());
        }
        if let Some(value) = config.server.get(key) {
            return Some(value.to_string());
        }
    }
    None
}

#[derive(Debug)]
pub struct App {
    pub command_input: String,
    pub screen: Screen,
    pub mode: Mode,
    pub logs: VecDeque<String>,
    /// How far the Logs screen is scrolled back, counted in lines hidden
    /// *below* the viewport. Zero means "follow the newest line", which is
    /// why it is the resting value rather than an offset from the top.
    pub log_scroll: usize,
    pub running: bool,
    /// Last terminal height we were told about, so the page keys can move
    /// by a real screenful. Updated only from `UiEvent::Resize`, which
    /// keeps `update` a pure state transition — it never asks the terminal
    /// anything, it is told.
    pub rows: u16,
    /// ...and the last width, for the one other piece of geometry `update`
    /// has to know: how many lines of the argv preview its pane hides.
    pub cols: u16,
    pub system: crate::services::system::SystemInfo,
    /// External programs probed before the production TUI starts. Kept in
    /// state so capability gates and `:about` report the same answer.
    pub tools: Tools,
    pub llama: LauncherState,
}

/// Is an ini value switched on? `no-mmproj = true` and a bare key both
/// count; anything else is off.
fn is_on(value: &str) -> bool {
    let value = value.trim();
    value.is_empty()
        || matches!(
            value.to_ascii_lowercase().as_str(),
            "true" | "on" | "yes" | "1"
        )
}

/// Terminal size assumed before the first `Resize`. The classic 24x80,
/// so a page is sane even if the size never arrives.
const DEFAULT_ROWS: u16 = 24;
const DEFAULT_COLS: u16 = 80;

impl App {
    /// Resolves the `models.ini` path itself (env / RAM tier / legacy).
    /// `main.rs` uses [`App::with_config_path`] instead so the same path is
    /// shared with the `Executor` and a `--config` flag can override it.
    pub fn new() -> Self {
        Self::with_config_path(llama::default_config_path())
    }

    pub fn with_config_path(config_path: PathBuf) -> Self {
        Self::restored(config_path, None, Prefs::default())
    }

    /// Builds the app with a remembered preset preselected, and whatever
    /// `~/.herd_config` remembers about favourites, overrides and the
    /// router.
    pub fn restored(config_path: PathBuf, last_launched: Option<String>, prefs: Prefs) -> Self {
        Self::restored_with_tools(config_path, last_launched, prefs, Tools::assumed())
    }

    pub fn restored_with_tools(
        config_path: PathBuf,
        last_launched: Option<String>,
        prefs: Prefs,
        tools: Tools,
    ) -> Self {
        let mut logs = VecDeque::with_capacity(MAX_LOGS);
        logs.push_back("HERD started".into());

        if let Err(error) = &tools.hf.version {
            logs.push_back(format!("downloads disabled: {error}"));
        }

        Self {
            command_input: String::new(),
            screen: Screen::Models,
            mode: Mode::Browse,
            logs,
            log_scroll: 0,
            running: false,
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            system: crate::services::system::SystemInfo::default(),
            tools,
            llama: LauncherState::new(config_path, last_launched, prefs),
        }
    }

    /// What to write to `~/.herd_config` on the way out.
    ///
    /// Taken here rather than saved as it changes, for the same reason the
    /// session file is: `App::update` is a pure state transition and does
    /// no I/O. The cost is that a `kill -9` loses the last few edits,
    /// which is the same bargain the session file already makes.
    pub fn prefs(&self) -> Prefs {
        Prefs {
            favorites: self.llama.favorites.clone(),
            mono_focus: self.llama.mono_focus.clone(),
            overrides: self.llama.overrides.clone(),
            router: self.llama.router.clone(),
        }
    }

    /// How far a page key moves on the current screen: one screenful of
    /// list, less a row of overlap so the line you were reading is still
    /// visible after the jump — the convention every pager follows, and
    /// the thing that makes paging feel continuous rather than teleporting.
    ///
    /// This replaced a flat 10. On a full-screen terminal that meant four
    /// presses to cross one screen; on a short one it jumped past rows the
    /// user never saw.
    pub fn page(&self) -> usize {
        crate::layout::page_rows(
            self.screen,
            ratatui::layout::Rect::new(0, 0, self.cols, self.rows),
        )
    }

    /// The argv the preview is showing on the current screen, wrapped to
    /// the pane it will be drawn in. `None` on a screen that has no
    /// preview, which is what makes the scroll keys inert there.
    fn preview_lines(&self) -> Option<usize> {
        let argv = match self.screen {
            Screen::Models => self.llama.argv_preview().ok()?,
            Screen::Router => self.llama.router_argv_preview().ok()?,
            _ => return None,
        };

        let terminal = ratatui::layout::Rect::new(0, 0, self.cols, self.rows);
        let (width, _) = crate::layout::preview_viewport(self.screen, terminal)?;
        Some(crate::components::wrap_argv(&argv, width).len())
    }

    /// How far the preview can be scrolled before the last line is on
    /// screen. Zero when it all fits, which is what stops the keys from
    /// advertising motion that never happens.
    pub fn preview_max_scroll(&self) -> usize {
        let terminal = ratatui::layout::Rect::new(0, 0, self.cols, self.rows);
        let visible = crate::layout::preview_viewport(self.screen, terminal)
            .map(|(_, height)| height)
            .unwrap_or(0);
        self.preview_lines().unwrap_or(0).saturating_sub(visible)
    }

    fn clamp_preview_scroll(&mut self) {
        self.llama.preview_scroll = self.llama.preview_scroll.min(self.preview_max_scroll());
    }

    /// Scrolls the argv preview, one line at a time.
    ///
    /// A long argv — a preset with the `[mono-focus]` profile on, say —
    /// wraps past the six rows the pane has, and until now the rest was
    /// simply cut off with nothing to say it existed. Clamped here rather
    /// than at draw time, so `render` stays a pure function of `App` and
    /// the counter cannot climb past what is actually hidden.
    fn scroll_preview(&mut self, down: bool) -> Action {
        let max = self.preview_max_scroll();
        let scroll = &mut self.llama.preview_scroll;

        *scroll = if down {
            (*scroll + 1).min(max)
        } else {
            scroll.saturating_sub(1)
        };
        Action::None
    }

    pub fn push_log(&mut self, entry: impl AsRef<str>) {
        for line in entry.as_ref().lines() {
            if self.logs.len() >= MAX_LOGS {
                self.logs.pop_front();
            }
            self.logs.push_back(line.to_string());

            // Someone reading back through the logs must not have the view
            // dragged off the line they are on every time the server emits
            // another one. Pinning means counting the new line too. At 0 —
            // following the tail — there is nothing to pin.
            if self.log_scroll > 0 {
                self.log_scroll = (self.log_scroll + 1).min(self.logs.len().saturating_sub(1));
            }
        }
    }

    pub fn update(&mut self, event: UiEvent) -> Action {
        match event {
            UiEvent::Key(key) => self.handle_key(key),
            UiEvent::Tick => Action::None,
            UiEvent::CommandFinished { command, output } => {
                self.running = false;
                self.push_log(format!(":{} -> {}", command, output));
                Action::None
            }
            UiEvent::Log(line) => {
                self.push_log(line);
                Action::None
            }
            UiEvent::LlamaStatus(snapshot) => {
                self.apply_status(snapshot);
                Action::None
            }
            UiEvent::SystemInfo(info) => {
                self.system = info;
                Action::None
            }
            UiEvent::AvailableMemory(available) => {
                self.system.available_memory_gib = available;
                Action::None
            }
            UiEvent::PortInUse { port, name, retry } => {
                self.ask(name, Confirm::PortInUse { port, retry });
                self.push_log(format!(
                    "port {port} is already in use by a process herd did not start"
                ));
                Action::None
            }
            UiEvent::ChatResult(result) => {
                self.llama.chat_pending = false;
                self.llama.chat_started = None;
                match result.as_ref() {
                    Ok(outcome) => {
                        self.llama.stats.record(outcome);
                        self.push_log(format!("test -> {}", outcome.summary()));
                    }
                    Err(error) => {
                        self.llama.stats.failures += 1;
                        self.push_log(format!("test -> error: {error}"));
                    }
                }
                self.llama.chat = Some(*result);
                Action::None
            }
            UiEvent::CacheList(cached) => {
                self.llama.cached = Some(cached);
                // The Hub screen lists exactly these entries, and a refresh
                // that finds fewer (a deletion, a tier with less in it)
                // must not leave its cursor pointing past the end.
                self.llama.clamp_hub_cursor();
                Action::None
            }
            UiEvent::DownloadProgress { model, done, total } => {
                self.llama.download = Some(Download { model, done, total });
                Action::None
            }
            UiEvent::DownloadFinished { model, result } => {
                self.llama.download = None;
                match result.as_ref() {
                    Ok(summary) => self.push_log(summary.clone()),
                    Err(error) => self.push_log(format!("download {model} failed: {error}")),
                }
                Action::None
            }
            UiEvent::Resize { width, height } => {
                self.rows = height;
                self.cols = width;
                // A narrower terminal wraps the argv into more lines and a
                // wider one into fewer, so a scroll offset taken at the
                // old size can now point past the end.
                self.clamp_preview_scroll();
                Action::None
            }
            UiEvent::Quit => Action::Quit,
        }
    }

    fn apply_status(&mut self, snapshot: llama::LlamaSnapshot) {
        self.llama.server.mode = snapshot.mode;

        // A snapshot without a model name (STOPPING, OFF) carries no new
        // identity — it must not erase the one already displayed, or the
        // Server screen would blank the name mid-transition.
        if snapshot.model.is_some() {
            self.llama.server.model = snapshot.model.clone();
        }

        match &snapshot.state {
            ServerState::Starting => {
                self.llama.server.started_at = Some(Instant::now());
                self.llama.stats.begin();
                self.llama.server.endpoint = self.endpoint();
                if let Some(model) = snapshot.model.clone() {
                    self.llama.last_launched = Some(model);
                }
            }
            // Nothing is loaded any more: drop the model, or its row keeps
            // the "serving" marker and the next launch looks like a no-op.
            ServerState::Off => {
                self.llama.server.started_at = None;
                self.llama.server.endpoint = None;
                self.llama.server.model = None;
            }
            // Errors keep the model name: "ERROR gemma4-12b" is the whole
            // point. The marker is driven by the state, so a dead model is
            // still not shown as serving.
            ServerState::Error(reason) => {
                self.llama.server.started_at = None;
                self.llama.server.endpoint = None;
                let reason = reason.clone();
                self.explain_failure(&reason);
            }
            ServerState::Serving | ServerState::Stopping => {}
        }

        self.llama.server.phase = snapshot.phase;
        self.llama.server.state = snapshot.state;
    }

    /// Adds the sizing context to a failed launch.
    ///
    /// The supervisor can say "killed by the system (SIGKILL) — most likely
    /// out of memory", but only the App knows what this preset was
    /// estimated to need and what the budget allowed. Those two numbers are
    /// what turn the message into a next step, so they are logged whenever
    /// the estimate was already a concern. Presets that fit comfortably —
    /// or that cannot be sized at all — say nothing, on the same principle
    /// as `Fit::Unknown`: never claim what you cannot support.
    fn explain_failure(&mut self, reason: &str) {
        let Some(model) = self.llama.server.model.clone() else {
            return;
        };
        if !matches!(self.llama.fit(&model), Fit::TooLarge | Fit::Tight) {
            return;
        }
        let Some(estimate) = self.llama.estimate_gib(&model) else {
            return;
        };

        self.push_log(format!(
            "{model} failed ({reason}); it is estimated at {estimate:.1} GiB against a \
             {:.1} GiB budget — try a smaller preset, a lower ctx-size, or raise the \
             budget on the Stats screen",
            self.llama.budget().available_gib()
        ));
    }

    fn endpoint(&self) -> Option<String> {
        let config = self.llama.config.as_ref()?;
        Some(llama::api::base_url(&config.client_host(), config.port()))
    }

    fn handle_key(&mut self, key: KeyEvent) -> Action {
        match self.mode {
            // All three overlays are reference cards, and all close on any
            // key: hunting for the one that dismisses a help box would be
            // its own small joke.
            Mode::Help | Mode::Commands | Mode::About => self.handle_help_key(key),
            Mode::ConfirmDelete => self.handle_delete_key(key),
            Mode::Command => self.handle_command_key(key),
            Mode::Filter => self.handle_filter_key(key),
            Mode::EditSetting => self.handle_edit_key(key),
            Mode::EditPrompt => self.handle_prompt_key(key),
            Mode::Picker => self.handle_picker_key(key),
            Mode::ConfirmLaunch => self.handle_confirm_key(key),
            Mode::ConfirmQuit => self.handle_quit_key(key),
            Mode::Browse => self.handle_browse_key(key),
        }
    }

    fn handle_browse_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('q') => self.quit(),
            // The force variant, in the same spirit as `launch!`: the
            // answer to the prompt, given before the prompt appears.
            KeyCode::Char('Q') => Action::Quit,
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                Action::None
            }
            // ←/→ alias Tab/Shift+Tab. They were dead keys in Browse, and
            // reaching for an arrow to move between screens is the first
            // thing anyone tries; ↑/↓ cannot be the pair because every
            // screen with a list needs them for the list.
            KeyCode::Tab | KeyCode::Right => {
                self.screen = self.screen.next();
                Action::None
            }
            KeyCode::BackTab | KeyCode::Left => {
                self.screen = self.screen.prev();
                Action::None
            }
            KeyCode::Char('c') => {
                self.llama.picker_cursor = self
                    .llama
                    .config_choices()
                    .iter()
                    .position(|choice| choice.path == self.llama.config_path)
                    .unwrap_or(0);
                self.mode = Mode::Picker;
                Action::None
            }
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
                Action::None
            }
            KeyCode::Char(c @ '1'..='8') => {
                let index = c as usize - '1' as usize;
                self.screen = Screen::ALL[index];
                Action::None
            }
            _ => screen_input::dispatch(self, key),
        }
    }

    /// Any key dismisses the overlay: it is a reference card, and hunting
    /// for the one key that closes it would be its own small joke.
    fn handle_help_key(&mut self, _key: KeyEvent) -> Action {
        self.mode = Mode::Browse;
        Action::None
    }

    fn handle_models_key(&mut self, key: KeyEvent) -> Action {
        if let Some(cursor) = screen_input::moved(
            self.llama.cursor,
            self.llama.rows().len(),
            key.code,
            self.page(),
        ) {
            self.llama.cursor = cursor;
            // The Settings screen shows the selected preset's keys, so its
            // cursor has to stay in range when the selection moves.
            self.llama.clamp_settings_cursor();
            // A new row means a new argv: staying scrolled into the middle
            // of the previous one would be reading the wrong command.
            self.llama.preview_scroll = 0;
            return Action::None;
        }

        match key.code {
            KeyCode::Char('J') => self.scroll_preview(true),
            KeyCode::Char('K') => self.scroll_preview(false),
            KeyCode::Char('/') => {
                self.mode = Mode::Filter;
                Action::None
            }
            KeyCode::Char('t') => self.switch_tier(true),
            KeyCode::Char('T') => self.switch_tier(false),
            KeyCode::Char('r') => {
                self.llama.reload();
                self.push_log(format!("reloaded {}", self.llama.config_path.display()));
                Action::None
            }
            KeyCode::Enter => self.launch_selected(),
            KeyCode::Char('d') => self.download_selected(),
            KeyCode::Char('y') => self.copy_selected(),
            KeyCode::Char('f') => self.star_selected(),
            KeyCode::Char('s') => self.stop_server(),
            _ => Action::None,
        }
    }

    /// Hub screen: what is in the cache, and what to do about it.
    ///
    /// The one destructive key in the program is here, and it is
    /// **uppercase `D`**. Lowercase `d` on the Models screen *downloads* —
    /// the opposite act — and moving between two screens where the same
    /// finger means "fetch this" and "destroy this" is precisely how an
    /// accident happens. Capitals are already the force variants here
    /// (`Q`, `X`), so the shift key carries its usual meaning.
    fn handle_hub_key(&mut self, key: KeyEvent) -> Action {
        if let Some(cursor) = screen_input::moved(
            self.llama.hub_cursor,
            self.llama.hub_rows().len(),
            key.code,
            self.page(),
        ) {
            self.llama.hub_cursor = cursor;
            return Action::None;
        }

        match key.code {
            KeyCode::Char('y') => self.copy_stanza(),
            KeyCode::Char('r') => self.run("cache".to_string()),
            KeyCode::Char('D') => self.ask_delete(),
            KeyCode::Enter => self.reveal_in_models(),
            _ => Action::None,
        }
    }

    /// Parks a deletion and asks about it, unless something makes it a bad
    /// idea outright.
    ///
    /// Two refusals rather than warnings, because neither has a sensible
    /// "yes anyway": pulling the weights out from under a server that is
    /// serving them, and deleting a directory another process is still
    /// writing into. Both would leave the machine in a state the user did
    /// not ask for and herd could not explain afterwards.
    fn ask_delete(&mut self) -> Action {
        let Some(row) = self.llama.selected_hub() else {
            return Action::None;
        };
        let repo = llama::hub::split_repo(&row.reference).0.to_string();

        if self.serving_repo().is_some_and(|serving| serving == repo) {
            self.push_log(format!(
                "{} is serving from {repo} — stop it before deleting the weights",
                self.llama.server.model.clone().unwrap_or_default()
            ));
            return Action::None;
        }

        if let Some(download) = &self.llama.download {
            if self
                .llama
                .repo_of(&download.model)
                .map(|reference| llama::hub::split_repo(&reference).0.to_string())
                .is_some_and(|downloading| downloading == repo)
            {
                self.push_log(format!(
                    "{repo} is being downloaded — wait for it to finish"
                ));
                return Action::None;
            }
        }

        // Everything else cached in the same directory goes too, so the
        // prompt has to name it before the question is answered.
        let also_removes = self
            .llama
            .hub_rows()
            .into_iter()
            .filter(|other| {
                other.reference != row.reference
                    && llama::hub::split_repo(&other.reference).0 == repo
            })
            .map(|other| other.reference)
            .collect();

        self.llama.pending_delete = Some(PendingDelete {
            reference: row.reference.clone(),
            repo,
            bytes: row.disk,
            also_removes,
        });
        self.mode = Mode::ConfirmDelete;

        Action::None
    }

    /// The repo the running server is serving from, if it is running.
    fn serving_repo(&self) -> Option<String> {
        if !self.llama.server.state.is_live() {
            return None;
        }
        let model = self.llama.server.model.as_deref()?;
        let reference = self.llama.repo_of(model)?;

        Some(llama::hub::split_repo(&reference).0.to_string())
    }

    fn handle_delete_key(&mut self, key: KeyEvent) -> Action {
        let pending = self.llama.pending_delete.take();
        self.mode = Mode::Browse;

        // Only a lowercase `y`, unlike the launch prompts. Those start
        // something; this one ends something, and a capital Y arriving
        // from a slipped shift key should not be what does it.
        match (key.code, pending) {
            (KeyCode::Char('y'), Some(pending)) => {
                self.push_log(format!("deleting {}", pending.repo));
                Action::DeleteModel {
                    reference: pending.reference,
                    repo: pending.repo,
                }
            }
            _ => {
                self.push_log("delete cancelled");
                Action::None
            }
        }
    }

    /// Copies a `[preset]` stanza for the highlighted cached model.
    ///
    /// This is the whole point of listing the cache: a model that is on the
    /// machine but in no tier is unusable, and the gap between the two is
    /// three lines of ini. Copying them beats reading a repo reference off
    /// a terminal and retyping it — which is where the typo that makes
    /// llama.cpp fetch a second copy comes from.
    fn copy_stanza(&mut self) -> Action {
        let Some(row) = self.llama.selected_hub() else {
            self.push_log("nothing to copy: the cache list is empty");
            return Action::None;
        };

        Action::CopyToClipboard {
            label: format!("{} preset", llama::ini::preset_name(&row.reference)),
            text: llama::ini::preset_stanza(&row.reference),
        }
    }

    /// Jumps to the highlighted model's preset on the Models screen, where
    /// everything that acts on a preset already lives. A cached model with
    /// no preset in this tier has nothing to jump to, and says so rather
    /// than moving the cursor somewhere arbitrary.
    fn reveal_in_models(&mut self) -> Action {
        let Some(row) = self.llama.selected_hub() else {
            return Action::None;
        };

        match row.preset.and_then(|preset| {
            self.llama
                .rows()
                .iter()
                .position(|candidate| candidate.name == preset)
        }) {
            Some(index) => {
                self.llama.cursor = index;
                self.llama.clamp_settings_cursor();
                self.screen = Screen::Models;
            }
            None => self.push_log(format!(
                "no preset in this tier names {} — copy a stanza with y",
                row.reference
            )),
        }

        Action::None
    }

    /// Router screen: two numbers and the process they would start.
    ///
    /// `+`/`-` rather than an edit mode, matching the Stats screen's
    /// memory reservation: both are single numbers with a sensible range,
    /// and typing one is more keystrokes than nudging it.
    fn handle_router_key(&mut self, key: KeyEvent) -> Action {
        if let Some(cursor) = screen_input::moved(
            self.llama.router_cursor,
            self.llama.router_rows().len(),
            key.code,
            self.page(),
        ) {
            self.llama.router_cursor = cursor;
            return Action::None;
        }

        match key.code {
            KeyCode::Char('J') => self.scroll_preview(true),
            KeyCode::Char('K') => self.scroll_preview(false),
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.llama.adjust_router(true);
                Action::None
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                self.llama.adjust_router(false);
                Action::None
            }
            KeyCode::Char('r') => {
                self.llama.router = RouterPrefs::default();
                self.push_log("router settings reset to the defaults");
                Action::None
            }
            KeyCode::Char('y') => self.copy_router(),
            KeyCode::Char('s') => self.stop_server(),
            // Always allowed, unlike the Server screen's Enter: the
            // numbers it starts with are the ones on screen, and having
            // just changed one is the reason to press it.
            KeyCode::Enter => self.run(format!(
                "router --max {} --idle {}",
                self.llama.router.models_max, self.llama.router.sleep_idle_seconds
            )),
            _ => Action::None,
        }
    }

    /// Stars the highlighted preset, or unstars it.
    fn star_selected(&mut self) -> Action {
        match self.llama.toggle_favorite() {
            Some((model, true)) => self.push_log(format!("{model} starred")),
            Some((model, false)) => self.push_log(format!("{model} unstarred")),
            None => {}
        }
        Action::None
    }

    /// Copies the router's launch command, exactly as `y` does on the
    /// Models screen — same question, same answer, same key.
    fn copy_router(&mut self) -> Action {
        match self.llama.router_shell_command() {
            Ok(text) => Action::CopyToClipboard {
                label: "router".into(),
                text,
            },
            Err(error) => {
                self.push_log(format!("nothing to copy: {error}"));
                Action::None
            }
        }
    }

    fn handle_server_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('s') => self.stop_server(),
            // A ping needs something to ping. With nothing running there
            // is no model name to send, and the key used to do nothing at
            // all — indistinguishable from a key that is not bound, or
            // from herd having ignored the press.
            KeyCode::Char('p') => match self.llama.server.model.clone() {
                Some(model) => self.run(format!("ping {model}")),
                None => {
                    self.push_log(
                        "nothing to ping — no llama-server is running \
                         (start one with enter, or :router)",
                    );
                    Action::None
                }
            },
            KeyCode::Enter => match self.llama.relaunch_blocked() {
                Some(reason) => {
                    self.push_log(reason);
                    Action::None
                }
                None => self.launch_selected(),
            },
            _ => Action::None,
        }
    }

    /// Test screen: send the prompt to whatever is loaded, or edit it.
    fn handle_test_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter => self.send_chat(),
            KeyCode::Char('e') => {
                self.llama.edit_buffer = self.llama.prompt.clone();
                self.mode = Mode::EditPrompt;
                Action::None
            }
            KeyCode::Char('r') => {
                self.llama.prompt = llama::api::DEFAULT_PROMPT.to_string();
                self.llama.chat = None;
                Action::None
            }
            _ => Action::None,
        }
    }

    /// A probe is only meaningful against a server that can answer, and
    /// only one at a time — a second Enter while one is in flight would
    /// otherwise silently queue a duplicate request.
    fn send_chat(&mut self) -> Action {
        if self.llama.chat_pending {
            self.push_log("a test is already in flight");
            return Action::None;
        }

        let Some(model) = self.llama.test_target() else {
            self.push_log("no model to test");
            return Action::None;
        };

        if !self.llama.server.state.is_live() {
            self.push_log(format!(
                "server is {}, start a model before testing",
                self.llama.server.state.tag()
            ));
            return Action::None;
        }

        self.llama.chat_pending = true;
        self.llama.chat_started = Some(Instant::now());
        self.llama.chat = None;
        self.push_log(format!("test -> {model}: {:?}", self.llama.prompt));

        Action::RunChat {
            model,
            prompt: self.llama.prompt.clone(),
        }
    }

    fn handle_prompt_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => {
                self.llama.edit_buffer.clear();
                self.mode = Mode::Browse;
                Action::None
            }
            KeyCode::Backspace => {
                self.llama.edit_buffer.pop();
                Action::None
            }
            KeyCode::Enter => {
                let prompt = self.llama.edit_buffer.trim().to_string();
                if !prompt.is_empty() {
                    self.llama.prompt = prompt;
                }
                self.llama.edit_buffer.clear();
                self.mode = Mode::Browse;
                Action::None
            }
            KeyCode::Char(c) => {
                self.llama.edit_buffer.push(c);
                Action::None
            }
            _ => Action::None,
        }
    }

    /// Stats screen: the only control is the memory reservation, because
    /// it is the one number here the user can actually change.
    fn handle_stats_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('+') | KeyCode::Char('=') => self.adjust_reserved(0.05),
            KeyCode::Char('-') | KeyCode::Char('_') => self.adjust_reserved(-0.05),
            KeyCode::Char('r') => {
                self.llama.reserved_ratio = memory::DEFAULT_RESERVED_RATIO;
                self.push_log("memory reservation reset to the default");
                Action::None
            }
            _ => Action::None,
        }
    }

    /// Lowering the reservation hands memory to the model that the OS
    /// expects to keep. It is allowed — it is the whole point of the
    /// override — but never silently: dropping under the default logs a
    /// warning, and the screen shows a standing caution.
    fn adjust_reserved(&mut self, delta: f64) -> Action {
        let before = self.llama.reserved_ratio;
        // Snapped to whole percent: repeated += 0.05 otherwise drifts, and
        // the bounds would never be reached exactly.
        let stepped = ((before + delta) * 100.0).round() / 100.0;
        let after = stepped.clamp(memory::MIN_RESERVED_RATIO, memory::MAX_RESERVED_RATIO);

        if (after - before).abs() < f64::EPSILON {
            return Action::None;
        }

        self.llama.reserved_ratio = after;

        if after < memory::DEFAULT_RESERVED_RATIO && before >= memory::DEFAULT_RESERVED_RATIO {
            self.push_log(
                "CAUTION: reserving less than the default leaves the OS short of memory; \
                 the machine may swap, stall or kill the server",
            );
        }
        self.push_log(format!(
            "memory reserved for the system: {:.0}% ({:.1} GiB of {:.1})",
            after * 100.0,
            self.llama.budget().reserved_gib(),
            self.llama.budget().total_gib
        ));

        Action::None
    }

    fn handle_picker_key(&mut self, key: KeyEvent) -> Action {
        let choices = self.llama.config_choices();

        // Esc/c first: the picker is a modal list, and `c` toggling it shut
        // must win over anything the shared movement set might claim.
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('c')) {
            self.mode = Mode::Browse;
            return Action::None;
        }

        if let Some(cursor) = screen_input::moved(
            self.llama.picker_cursor,
            choices.len(),
            key.code,
            self.page(),
        ) {
            self.llama.picker_cursor = cursor;
            return Action::None;
        }

        match key.code {
            KeyCode::Enter => {
                self.mode = Mode::Browse;

                let Some(choice) = choices.get(self.llama.picker_cursor) else {
                    return Action::None;
                };
                if choice.path == self.llama.config_path {
                    return Action::None;
                }

                self.llama.config_path = choice.path.clone();
                self.llama.cursor = 0;
                self.llama.settings_cursor = 0;
                self.llama.reload();
                self.llama.restore_cursor();

                if choice.too_large > 0 {
                    self.push_log(format!(
                        "WARNING: {} of {} presets in {} exceed this machine's {:.1} GiB budget",
                        choice.too_large,
                        choice.presets,
                        choice.label,
                        self.llama.budget().available_gib()
                    ));
                }
                self.push_log(format!("config -> {}", self.llama.config_path.display()));

                Action::ConfigPathChanged(self.llama.config_path.clone())
            }
            _ => Action::None,
        }
    }

    /// Logs move the opposite way round: the buffer grows downwards and
    /// the resting position is the newest line, so `log_scroll` counts
    /// *backwards* from the tail and "move down" walks towards it.
    fn handle_logs_key(&mut self, key: KeyEvent) -> Action {
        let oldest = self.logs.len().saturating_sub(1);
        let page = self.page();

        self.log_scroll = match key.code {
            KeyCode::Up | KeyCode::Char('k') => (self.log_scroll + 1).min(oldest),
            KeyCode::Down | KeyCode::Char('j') => self.log_scroll.saturating_sub(1),
            KeyCode::PageUp => (self.log_scroll + page).min(oldest),
            KeyCode::PageDown => self.log_scroll.saturating_sub(page),
            KeyCode::Home | KeyCode::Char('g') => oldest,
            KeyCode::End | KeyCode::Char('G') => 0,
            _ => return Action::None,
        };

        Action::None
    }

    fn handle_settings_key(&mut self, key: KeyEvent) -> Action {
        if let Some(cursor) = screen_input::moved(
            self.llama.settings_cursor,
            self.llama.setting_entry_indices().len(),
            key.code,
            self.page(),
        ) {
            self.llama.settings_cursor = cursor;
            return Action::None;
        }

        match key.code {
            KeyCode::Enter => self.edit_or_toggle_setting(),
            // The profile is switched from here rather than from a row:
            // "is this on" is not a `key = value` in the file, and giving
            // it a fake row would make it look like one.
            KeyCode::Char('m') => self.switch_mono_focus(),
            KeyCode::Char('x') => {
                if let Some(row) = self.llama.selected_setting() {
                    if let Some((scope, model, key, _, _)) = row.as_entry() {
                        self.llama.overrides.clear(*scope, model, key);
                    }
                }
                Action::None
            }
            KeyCode::Char('X') => {
                if !self.llama.overrides.is_empty() {
                    let count = self.llama.overrides.count();
                    self.llama.overrides.clear_all();
                    self.push_log(format!("cleared {count} session override(s)"));
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    /// `m` on the Settings screen: switch the `[mono-focus]` profile for
    /// the highlighted preset, and say what that means rather than only
    /// that it happened — the flags are the point of it.
    fn switch_mono_focus(&mut self) -> Action {
        match self.llama.toggle_mono_focus() {
            Some((model, true)) => {
                let flags = self
                    .llama
                    .config
                    .as_ref()
                    .map(|config| {
                        config
                            .mono_focus
                            .iter()
                            .map(|(key, value)| format!("{key}={value}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                self.push_log(format!("mono-focus on for {model}: {flags}"));
            }
            Some((model, false)) => self.push_log(format!("mono-focus off for {model}")),
            None => self.push_log(format!(
                "no [{}] section in {} — nothing to switch on",
                llama::ini::MONO_FOCUS,
                self.llama.config_path.display()
            )),
        }
        // The cursor counts entries, and switching it on adds some.
        self.llama.clamp_settings_cursor();
        Action::None
    }

    /// Enter on a setting: flip it if it is a boolean, otherwise open the
    /// editor on it.
    ///
    /// Typing `false` over `true` is four keystrokes and a chance to
    /// mistype, for a value that has exactly one other possibility. So the
    /// values with only one other possibility skip the editor entirely.
    /// Everything else — ports, context sizes, repo names — still goes
    /// through it, and `x` restores the ini value if a toggle was not what
    /// was wanted.
    fn edit_or_toggle_setting(&mut self) -> Action {
        let Some(row) = self.llama.selected_setting() else {
            return Action::None;
        };
        let Some((scope, model, key, ini_value, override_value)) = row.as_entry() else {
            return Action::None;
        };

        let current = override_value.unwrap_or(ini_value);

        match llama::overrides::toggled(current) {
            Some(next) => {
                let (scope, model, key) = (*scope, model.to_string(), key.to_string());
                self.llama.overrides.set(scope, &model, &key, &next);
                self.push_log(format!("{key} {current} -> {next}"));
                Action::None
            }
            None => {
                self.llama.edit_buffer = current.to_string();
                self.mode = Mode::EditSetting;
                Action::None
            }
        }
    }

    fn handle_command_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => {
                self.command_input.clear();
                self.mode = Mode::Browse;
                Action::None
            }
            KeyCode::Backspace => {
                self.command_input.pop();
                Action::None
            }
            KeyCode::Enter => self.submit_command(),
            KeyCode::Char(c) => {
                self.command_input.push(c);
                Action::None
            }
            _ => Action::None,
        }
    }

    fn handle_filter_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => {
                self.llama.filter.clear();
                self.llama.clamp_cursor();
                self.mode = Mode::Browse;
                Action::None
            }
            KeyCode::Enter => {
                self.mode = Mode::Browse;
                Action::None
            }
            KeyCode::Backspace => {
                self.llama.filter.pop();
                self.llama.clamp_cursor();
                Action::None
            }
            KeyCode::Char(c) => {
                self.llama.filter.push(c);
                self.llama.cursor = 0;
                Action::None
            }
            _ => Action::None,
        }
    }

    fn handle_edit_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => {
                self.llama.edit_buffer.clear();
                self.mode = Mode::Browse;
                Action::None
            }
            KeyCode::Backspace => {
                self.llama.edit_buffer.pop();
                Action::None
            }
            KeyCode::Enter => {
                let value = self.llama.edit_buffer.clone();
                if let Some(row) = self.llama.selected_setting() {
                    if let Some((scope, model, key, ini_value, _)) = row.as_entry() {
                        // Re-entering the ini value clears the override
                        // instead of pinning an identical one.
                        if value == ini_value {
                            self.llama.overrides.clear(*scope, model, key);
                        } else {
                            self.llama.overrides.set(*scope, model, key, &value);
                        }
                    }
                }
                self.llama.edit_buffer.clear();
                self.mode = Mode::Browse;
                Action::None
            }
            KeyCode::Char(c) => {
                self.llama.edit_buffer.push(c);
                Action::None
            }
            _ => Action::None,
        }
    }

    /// Everything that would be thrown away by quitting right now, named.
    ///
    /// A supervised llama-server is deliberately **not** in this list: it
    /// is stopped on exit by design, every time, and asking about the
    /// normal case would train the user to dismiss the prompt without
    /// reading it. Only work that would be *lost* counts.
    pub fn in_flight(&self) -> Vec<String> {
        let mut work = Vec::new();

        if let Some(download) = &self.llama.download {
            work.push(format!(
                "downloading {} ({})",
                download.model,
                download.label()
            ));
        }
        if self.llama.chat_pending {
            work.push("a test request is waiting for the model".to_string());
        }
        if self.running {
            work.push("a command is still running".to_string());
        }

        work
    }

    /// `q`: asks only while a supervised server is live. `Q` is immediate.
    fn quit(&mut self) -> Action {
        if self.llama.server.state.is_live() {
            self.mode = Mode::ConfirmQuit;
            Action::None
        } else {
            Action::Quit
        }
    }

    fn handle_quit_key(&mut self, key: KeyEvent) -> Action {
        self.mode = Mode::Browse;

        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Action::Quit,
            _ => {
                self.push_log("quit cancelled");
                Action::None
            }
        }
    }

    /// Parks a launch and asks the user about it.
    fn ask(&mut self, model: String, reason: Confirm) {
        self.llama.pending_launch = Some(model);
        self.llama.confirm = Some(reason);
        self.mode = Mode::ConfirmLaunch;
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> Action {
        let model = self.llama.pending_launch.take();
        let reason = self.llama.confirm.take();
        self.mode = Mode::Browse;

        let confirmed = matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'));

        match (confirmed, model, reason) {
            // The force variant skips the port check, which is exactly and
            // only what the user answered here. It comes back verbatim
            // from the Executor that refused it (`launch! …`, `router! …`)
            // rather than being rebuilt out of the display name, so a
            // launch with extra args or a router with its numbers retries
            // as exactly what was refused.
            (true, _, Some(Confirm::PortInUse { retry, .. })) => self.run(retry),
            // Fetch it, then launch it: the user asked for the model, not
            // for a download.
            (true, Some(model), Some(Confirm::NotDownloaded { repo })) => {
                self.start_download(model, repo, true)
            }
            // A confirmed oversized launch is still a plain `launch`: the
            // user accepted the memory risk, not a busy port, and must
            // still be asked if the port turns out to be taken too.
            (true, Some(model), _) => self.run(format!("launch {model}")),
            _ => {
                self.push_log("launch cancelled");
                Action::None
            }
        }
    }

    /// Switches tier and tells the caller to keep the Executor in step.
    /// A no-op tier change (only one tier, or none) reports nothing.
    fn switch_tier(&mut self, forward: bool) -> Action {
        let before = self.llama.config_path.clone();
        self.llama.cycle_tier(forward);

        if self.llama.config_path == before {
            return Action::None;
        }

        self.push_log(format!("tier -> {}", self.llama.config_path.display()));
        Action::ConfigPathChanged(self.llama.config_path.clone())
    }

    fn launch_selected(&mut self) -> Action {
        let Some(model) = self.llama.selected_model() else {
            return Action::None;
        };

        // Nothing to launch until the weights exist. Several gigabytes
        // over a domestic connection is not something to start because
        // someone pressed Enter on a row.
        if self.llama.availability(&model) == Availability::Missing {
            if let Some(repo) = self.llama.repo_of(&model) {
                self.push_log(format!("{model} is not available locally ({repo})"));
                self.ask(model, Confirm::NotDownloaded { repo });
                return Action::None;
            }
        }

        // The Models screen has flagged this preset as too large all along;
        // until now that warning was decoration. Launching anyway is the
        // fastest way to wedge a machine with no memory headroom, so it is
        // worth one keystroke of friction. A typed `:launch` is left alone
        // — that is an explicit instruction, not a highlighted row.
        if self.llama.fit(&model) == Fit::TooLarge {
            if let Some(estimate) = self.llama.estimate_gib(&model) {
                let budget = self.llama.budget().available_gib();
                self.push_log(format!(
                    "{model} is estimated at {estimate:.1} GiB, over the {budget:.1} GiB budget"
                ));
                self.ask(model, Confirm::TooLarge { estimate, budget });
                return Action::None;
            }
        }

        self.run(format!("launch {model}"))
    }

    /// Copies the highlighted preset's launch command to the clipboard.
    ///
    /// The argv preview has always been the answer to "what would this
    /// actually run?", and the answer was only ever readable, never
    /// usable: reproducing it meant retyping twenty flags out of a pane in
    /// an alt-screen the mouse cannot select cleanly. The copy is the same
    /// argv the launch would spawn — overrides included — quoted onto one
    /// line, so it can be pasted into a shell, a script or a bug report.
    ///
    /// Deliberately outside the `running` flag: putting a line of text on
    /// the clipboard is not work the UI waits for, and it must stay
    /// available while a launch or a download is in flight — a command
    /// that failed is exactly when someone wants to run it by hand.
    fn copy_selected(&mut self) -> Action {
        let model = self.llama.selected_model().unwrap_or_else(|| "-".into());

        match self.llama.shell_command() {
            Ok(text) => Action::CopyToClipboard { label: model, text },
            Err(error) => {
                self.push_log(format!("nothing to copy: {error}"));
                Action::None
            }
        }
    }

    /// Fetches the highlighted preset without launching it.
    ///
    /// Worth its own key rather than only happening on the way to a
    /// launch: a fresh tier can be mostly empty, and queueing the
    /// downloads you know you will want beats discovering each one at the
    /// moment you wanted to use it.
    fn download_selected(&mut self) -> Action {
        let Some(model) = self.llama.selected_model() else {
            return Action::None;
        };

        if self.llama.availability(&model) == Availability::Local {
            self.push_log(format!("{model} is already available locally"));
            return Action::None;
        }

        match self.llama.repo_of(&model) {
            Some(repo) => self.start_download(model, repo, false),
            None => {
                self.push_log(format!("{model} names no hf-repo to download"));
                Action::None
            }
        }
    }

    /// One download at a time: two `hf` processes writing into the same
    /// cache is a fight nobody needs, and the screen has one bar.
    fn start_download(&mut self, model: String, repo: String, then_launch: bool) -> Action {
        if !self.tools.hf.available() {
            self.push_log(format!(
                "cannot download {model}: {}",
                self.tools.hf.label()
            ));
            return Action::None;
        }

        if let Some(running) = &self.llama.download {
            self.push_log(format!(
                "already downloading {}, wait for it to finish",
                running.model
            ));
            return Action::None;
        }

        let wants = self.llama.wants(&model);
        self.llama.download = Some(Download {
            model: model.clone(),
            done: 0,
            total: 0,
        });
        self.push_log(format!("downloading {repo}"));

        Action::Download {
            model,
            repo,
            wants,
            then_launch,
        }
    }

    fn stop_server(&mut self) -> Action {
        if self.llama.server.state.is_live() {
            return self.dispatch_stop();
        }

        // Nothing is running, so there is nothing to stop — but a failed
        // launch leaves ERROR on screen with no key that clears it, and
        // `s` is the only one that could plausibly mean "I have read
        // that, put it away". Purely local: no process is involved.
        if let ServerState::Error(reason) = self.llama.server.state.clone() {
            self.llama.server.state = ServerState::Off;
            self.llama.server.phase = Phase::None;
            self.llama.server.model = None;
            self.push_log(format!("cleared: {reason}"));
        }

        Action::None
    }

    /// Queues `:stop`, **bypassing the busy gate**.
    ///
    /// Stop is the one command whose whole purpose is to unwedge things, so
    /// gating it on nothing else being in flight had it refuse precisely
    /// when it was needed: a launch that was slow to spawn or slow to be
    /// killed held `running`, and every stop keypress was answered with
    /// "busy, ignored :stop". It is idempotent and the `Supervisor` is the
    /// real source of truth for whether anything is supervised, so letting
    /// it through costs nothing. It deliberately does not set `running`
    /// either — that flag belongs to the command it would otherwise steal
    /// the completion of.
    fn dispatch_stop(&mut self) -> Action {
        self.push_log("queued :stop");
        Action::RunCommand("stop".to_string())
    }

    /// Queues an async command, unless one is already in flight.
    ///
    /// Dropping the request silently is what makes a stop-then-launch feel
    /// broken: the stop is still running, the launch keypress vanishes,
    /// and nothing on screen explains why. Say so instead.
    fn run(&mut self, command: String) -> Action {
        if self.running {
            self.push_log(format!("busy, ignored :{command}"));
            return Action::None;
        }
        self.running = true;
        self.push_log(format!("queued :{command}"));
        Action::RunCommand(command)
    }

    fn submit_command(&mut self) -> Action {
        let command = self.command_input.trim().to_string();

        if command.is_empty() {
            return Action::None;
        }

        // Same reasoning as the `x` shortcut, and checked *before* the busy
        // gate: a typed `:stop` must work while something else is in
        // flight, or it is useless exactly when it matters.
        if command == "stop" {
            self.command_input.clear();
            self.mode = Mode::Browse;
            return self.dispatch_stop();
        }

        // `help` and `about` are answered here rather than dispatched, for
        // the same reason as `models` below: they are local text, and
        // there is nothing to run. They also have to work while something
        // is in flight — "what can I type" and "what am I running" are
        // questions people ask *because* they are stuck — so they are
        // checked before the busy gate, like `stop`.
        let overlay = match command.as_str() {
            "help" => Some(Mode::Commands),
            "about" => Some(Mode::About),
            _ => None,
        };
        if let Some(mode) = overlay {
            self.command_input.clear();
            self.mode = mode;
            return Action::None;
        }

        if self.running {
            return Action::None;
        }

        self.command_input.clear();
        self.mode = Mode::Browse;

        // `models`/`reload` are a synchronous local file read — handled
        // directly here rather than round-tripping through the Executor,
        // since there is nothing to run asynchronously.
        if command == "models" || command == "reload" {
            self.llama.reload();
            let summary = match &self.llama.config_error {
                Some(error) => {
                    format!(
                        "failed to load {}: {error}",
                        self.llama.config_path.display()
                    )
                }
                None => format!(
                    "loaded {} model(s) from {}",
                    self.llama.model_names().len(),
                    self.llama.config_path.display()
                ),
            };
            self.push_log(format!(":{command} -> {summary}"));
            return Action::None;
        }

        self.run(command)
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::llama::LlamaSnapshot;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use std::io::Write;

    fn key(code: KeyCode) -> UiEvent {
        UiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ch(c: char) -> UiEvent {
        key(KeyCode::Char(c))
    }

    /// Presses the digit that jumps to `screen`, worked out from
    /// `Screen::ALL` rather than written down. The digits are positional,
    /// so inserting a screen renumbers every one after it — and a test
    /// that hard-codes `3` does not fail then, it quietly starts testing a
    /// different screen.
    fn jump(app: &mut App, screen: Screen) {
        let index = Screen::ALL
            .iter()
            .position(|&candidate| candidate == screen)
            .expect("screen is in the table");
        let digit = char::from_digit(index as u32 + 1, 10).expect("a digit per screen");

        app.update(ch(digit));
    }

    const SAMPLE_INI: &str = r#"
[server]
host = 0.0.0.0
port = 1234
jinja = true

[*]
ctx-size = 32768
gpu-layers = 99

[gemma4-12b]
hf-repo = unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL
alias = gemma4-12b
spec-type = draft-mtp

[qwen3-coder]
hf-repo = unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:UD-Q4_K_XL
alias = qwen3-coder
"#;

    /// Path to a tier shipped in `data/`, for tests that need real
    /// preset lists rather than the two-model sample.
    fn shipped(tier: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join(tier)
            .join("models.ini")
    }

    /// Writes a throwaway models.ini and builds an App on it, so tests
    /// never depend on the developer's real ~/models layout.
    fn app_with_sample() -> App {
        let path = std::env::temp_dir().join(format!(
            "herd-app-{}-{:?}.ini",
            std::process::id(),
            std::thread::current().id()
        ));
        let mut file = std::fs::File::create(&path).expect("create sample ini");
        file.write_all(SAMPLE_INI.as_bytes()).expect("write ini");
        let mut app = App::with_config_path(path);
        // ...and pin the RAM too, for the same reason. `App::with_config_path`
        // reads the *real* machine's memory, which silently made every
        // fit-sensitive test built on this fixture depend on whose laptop it
        // ran on: `qwen3-coder` is a 30B and sizes at ~18 GiB, so it fits the
        // 36 GiB Mac these tests were written against and trips
        // `Confirm::TooLarge` on a 16 GiB one — where `enter_launches_the_
        // highlighted_preset` then failed, Enter having opened a prompt
        // rather than launched. Tests that care about a particular budget
        // still set `ram_gib` themselves; this is only a deterministic floor
        // under the ones that do not.
        app.llama.ram_gib = Some(36);
        app
    }

    /// Everything a keystroke could plausibly change, as a string. Used
    /// to detect that a key *did something* without needing `Eq` on `App`.
    fn fingerprint(app: &App) -> String {
        format!(
            "{:?}|{:?}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            app.screen,
            app.mode,
            app.command_input,
            app.running,
            app.logs.len(),
            app.log_scroll,
            app.llama.cursor,
            app.llama.settings_cursor,
            app.llama.picker_cursor,
            app.llama.filter,
            app.llama.prompt,
            app.llama.edit_buffer,
            app.llama.overrides.count(),
            app.llama.config_path.display(),
        )
    }

    /// The keymap in `keys.rs` is only worth having if it is complete.
    ///
    /// Drives every key into every screen and fails on any that changes
    /// the app — or returns an `Action` — without a binding naming it.
    /// This is what stops the table drifting from the match arms the way
    /// the hand-written hint strings did.
    #[test]
    fn every_key_that_does_something_is_documented() {
        let mut codes: Vec<KeyCode> = ('a'..='z')
            .chain('A'..='Z')
            .chain('0'..='9')
            .chain("+-=_/:?*.,;'\"[]{}()<>!@#$%^&|\\~`".chars())
            .map(KeyCode::Char)
            .collect();
        codes.extend([
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Backspace,
        ]);

        for screen in Screen::ALL {
            for code in &codes {
                let mut app = app_with_sample();
                app.screen = screen;
                // Something to scroll, so the Logs keys have an effect.
                app.push_log("a line\nanother line\na third line");

                let before = fingerprint(&app);
                let action = app.update(key(*code));
                let did_something = fingerprint(&app) != before || !matches!(action, Action::None);

                if !did_something {
                    continue;
                }

                let token = crate::keys::token(KeyEvent::new(*code, KeyModifiers::NONE))
                    .unwrap_or_else(|| panic!("{code:?} has no canonical name"));

                assert!(
                    crate::keys::documents(screen, &token),
                    "{screen:?} handles {token:?} but no binding in keys.rs documents it"
                );
            }
        }
    }

    #[test]
    fn page_and_edge_keys_move_further_than_one_row() {
        let mut app = App::with_config_path(shipped("16gb"));
        // A short terminal, so the page is smaller than the fixture and
        // the assertions below can tell paging apart from End.
        app.update(UiEvent::Resize {
            width: 120,
            height: 22,
        });
        let page = app.page();
        let last = app.llama.rows().len() - 1;
        assert!(last > page, "fixture needs more presets than a page");

        app.update(key(KeyCode::PageDown));
        assert_eq!(app.llama.cursor, page);

        app.update(key(KeyCode::End));
        assert_eq!(app.llama.cursor, last);

        app.update(key(KeyCode::PageUp));
        assert_eq!(app.llama.cursor, last - page);

        app.update(ch('g'));
        assert_eq!(app.llama.cursor, 0);

        app.update(ch('G'));
        assert_eq!(app.llama.cursor, last);
    }

    /// The page used to be a flat 10 rows whatever the terminal. On a
    /// full-screen window that was four presses to cross one screen; on a
    /// short one it jumped past rows the user never saw.
    #[test]
    fn a_page_follows_the_terminal_height() {
        let mut app = app_with_sample();

        app.update(UiEvent::Resize {
            width: 120,
            height: 24,
        });
        let short = app.page();

        app.update(UiEvent::Resize {
            width: 120,
            height: 60,
        });
        let tall = app.page();

        assert!(
            tall > short,
            "a taller terminal must page further: {short} vs {tall}"
        );
    }

    /// Even a terminal too small to hold a list at all must page by
    /// something, or the key silently does nothing.
    #[test]
    fn a_tiny_terminal_still_pages_by_something() {
        let mut app = app_with_sample();
        app.update(UiEvent::Resize {
            width: 120,
            height: 1,
        });

        assert!(app.page() >= 3, "page collapsed to {}", app.page());
    }

    /// Paging is computed by the same constraints as rendering, and must
    /// remain usable at ordinary and tiny terminal sizes.
    #[test]
    fn every_screen_gets_a_sane_page_from_the_shared_layout() {
        for screen in Screen::ALL {
            let page = crate::layout::page_rows(screen, ratatui::layout::Rect::new(0, 0, 120, 40));
            assert!(page >= 3, "{screen:?} collapsed to a {page}-row page");
        }
    }

    /// ←/→ were dead keys in Browse, and reaching for an arrow to move
    /// between screens is the first thing anyone tries.
    #[test]
    fn left_and_right_move_between_screens() {
        let mut app = app_with_sample();
        let first = app.screen;

        app.update(key(KeyCode::Right));
        assert_eq!(app.screen, first.next());

        app.update(key(KeyCode::Left));
        assert_eq!(app.screen, first, "left must undo right");

        app.update(key(KeyCode::Left));
        assert_eq!(app.screen, first.prev(), "and wrap the other way");
    }

    /// The picker hand-rolled its own `j`/`k` pair and so quietly lacked
    /// page, home and end — the one list in the app that answered to a
    /// different set of keys from every other.
    #[test]
    fn the_picker_answers_to_the_same_movement_keys_as_every_list() {
        let mut app = App::with_config_path(shipped("16gb"));
        app.mode = Mode::Picker;
        let last = app.llama.config_choices().len().saturating_sub(1);

        app.update(key(KeyCode::End));
        assert_eq!(app.llama.picker_cursor, last, "End must reach the last row");

        app.update(key(KeyCode::Home));
        assert_eq!(app.llama.picker_cursor, 0);

        app.update(key(KeyCode::PageDown));
        assert_eq!(
            app.llama.picker_cursor, last,
            "PageDown must clamp, not run off"
        );

        // Still a modal: `c` closes it rather than being eaten as movement.
        app.update(ch('c'));
        assert_eq!(app.mode, Mode::Browse);
    }

    /// Movement must stop at the ends rather than wrapping or running off
    /// into a row that does not exist.
    #[test]
    fn paging_clamps_at_both_ends() {
        let mut app = app_with_sample();

        app.update(key(KeyCode::PageDown));
        assert_eq!(app.llama.cursor, app.llama.rows().len() - 1);

        app.update(key(KeyCode::PageUp));
        assert_eq!(app.llama.cursor, 0);
    }

    #[test]
    fn an_empty_list_swallows_movement_without_panicking() {
        let mut app = App::with_config_path(PathBuf::from("/nonexistent/models.ini"));

        for code in [
            KeyCode::Down,
            KeyCode::PageDown,
            KeyCode::End,
            KeyCode::Home,
        ] {
            app.update(key(code));
            assert_eq!(app.llama.cursor, 0);
        }
    }

    #[test]
    fn the_help_overlay_opens_and_any_key_closes_it() {
        let mut app = app_with_sample();

        app.update(ch('?'));
        assert_eq!(app.mode, Mode::Help);

        // Even 'q': the overlay is a reference card, not a trap.
        app.update(ch('q'));
        assert_eq!(app.mode, Mode::Browse);
    }

    #[test]
    fn help_can_be_opened_from_any_screen() {
        for screen in Screen::ALL {
            let mut app = app_with_sample();
            app.screen = screen;
            app.update(ch('?'));
            assert_eq!(app.mode, Mode::Help, "{screen:?} would not open help");
        }
    }

    #[test]
    fn the_logs_screen_scrolls_back_and_returns_to_the_tail() {
        let mut app = app_with_sample();
        app.screen = Screen::Logs;
        for i in 0..40 {
            app.push_log(format!("line {i}"));
        }

        let page = app.page();

        app.update(ch('k'));
        assert_eq!(app.log_scroll, 1);

        app.update(key(KeyCode::PageUp));
        assert_eq!(app.log_scroll, 1 + page);

        app.update(ch('j'));
        assert_eq!(app.log_scroll, page);

        app.update(ch('G'));
        assert_eq!(app.log_scroll, 0, "G returns to the newest line");

        app.update(ch('g'));
        assert_eq!(app.log_scroll, app.logs.len() - 1, "g reaches the oldest");
    }

    /// The reason for scrolling back is usually to read the line where a
    /// launch failed — while the server is still writing more lines. The
    /// view has to stay on it.
    #[test]
    fn a_scrolled_back_log_view_stays_on_the_same_lines() {
        let mut app = app_with_sample();
        app.screen = Screen::Logs;
        for i in 0..40 {
            app.push_log(format!("line {i}"));
        }

        app.update(key(KeyCode::PageUp));
        let pinned = app.log_scroll;

        app.push_log("a line the server just emitted");
        assert_eq!(
            app.log_scroll,
            pinned + 1,
            "the view drifted off the pinned line"
        );
    }

    #[test]
    fn following_the_tail_is_not_disturbed_by_new_lines() {
        let mut app = app_with_sample();
        app.push_log("something happened");

        assert_eq!(app.log_scroll, 0);
    }

    #[test]
    fn q_quits_directly_when_no_server_is_live() {
        let mut app = app_with_sample();
        assert!(matches!(app.update(ch('q')), Action::Quit));
        assert_eq!(app.mode, Mode::Browse);
    }

    #[test]
    fn q_is_typed_when_the_command_bar_has_focus() {
        let mut app = app_with_sample();
        app.update(ch(':'));
        app.update(ch('q'));
        assert_eq!(app.command_input, "q");
        assert_eq!(app.mode, Mode::Command);
    }

    #[test]
    fn tab_cycles_screens_and_wraps() {
        let mut app = app_with_sample();
        assert_eq!(app.screen, Screen::Models);

        // Walked off `Screen::ALL` rather than written out: a screen
        // inserted in the middle must not need this test edited to keep
        // meaning "Tab visits every screen, in order, and wraps".
        for expected in Screen::ALL.into_iter().skip(1).chain([Screen::Models]) {
            app.update(key(KeyCode::Tab));
            assert_eq!(app.screen, expected);
        }
    }

    #[test]
    fn digits_jump_straight_to_a_screen() {
        let mut app = app_with_sample();

        for (index, screen) in Screen::ALL.into_iter().enumerate() {
            let digit = char::from_digit(index as u32 + 1, 10).expect("a digit per screen");
            app.update(ch(digit));
            assert_eq!(app.screen, screen, "{digit} went to the wrong screen");
        }
    }

    #[test]
    fn models_are_parsed_into_rows_with_their_settings() {
        let app = app_with_sample();
        let rows = app.llama.rows();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "gemma4-12b");
        assert_eq!(rows[0].repo, "unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL");
        // ctx-size is inherited from [*], not declared per model.
        assert_eq!(rows[0].ctx, "32768");
        assert_eq!(rows[0].spec, "mtp");
        assert_eq!(rows[1].spec, "-");
    }

    #[test]
    fn cursor_moves_and_clamps_at_both_ends() {
        let mut app = app_with_sample();
        assert_eq!(app.llama.cursor, 0);

        app.update(key(KeyCode::Up));
        assert_eq!(app.llama.cursor, 0, "clamps at the top");

        app.update(key(KeyCode::Down));
        assert_eq!(app.llama.cursor, 1);

        app.update(key(KeyCode::Down));
        assert_eq!(app.llama.cursor, 1, "clamps at the bottom");
    }

    #[test]
    fn enter_launches_the_highlighted_preset() {
        let mut app = app_with_sample();
        app.update(key(KeyCode::Down));

        match app.update(key(KeyCode::Enter)) {
            Action::RunCommand(command) => assert_eq!(command, "launch qwen3-coder"),
            other => panic!("expected RunCommand, got {other:?}"),
        }
        assert!(app.running);
    }

    #[test]
    fn filter_narrows_the_list_and_survives_enter() {
        let mut app = app_with_sample();
        app.update(ch('/'));
        assert_eq!(app.mode, Mode::Filter);

        for c in "qwen".chars() {
            app.update(ch(c));
        }
        assert_eq!(app.llama.rows().len(), 1);

        app.update(key(KeyCode::Enter));
        assert_eq!(app.mode, Mode::Browse);
        assert_eq!(app.llama.rows().len(), 1, "filter persists after Enter");
    }

    #[test]
    fn esc_clears_the_filter() {
        let mut app = app_with_sample();
        app.update(ch('/'));
        app.update(ch('z'));
        assert!(app.llama.rows().is_empty());

        app.update(key(KeyCode::Esc));
        assert_eq!(app.llama.rows().len(), 2);
        assert_eq!(app.mode, Mode::Browse);
    }

    /// A filter that hides the cursor's row must not leave the cursor
    /// pointing past the end of the list.
    #[test]
    fn filtering_clamps_the_cursor() {
        let mut app = app_with_sample();
        app.update(key(KeyCode::Down));
        assert_eq!(app.llama.cursor, 1);

        app.update(ch('/'));
        for c in "gemma".chars() {
            app.update(ch(c));
        }
        assert_eq!(app.llama.cursor, 0);
        assert_eq!(app.llama.selected_model().as_deref(), Some("gemma4-12b"));
    }

    #[test]
    fn argv_preview_includes_every_precedence_layer() {
        let app = app_with_sample();
        let argv = app.llama.argv_preview().expect("preview");

        assert!(argv.windows(2).any(|w| w == ["--port", "1234"]), "[server]");
        assert!(
            argv.windows(2).any(|w| w == ["--ctx-size", "32768"]),
            "[*] defaults"
        );
        assert!(
            argv.windows(2).any(|w| w == ["--alias", "gemma4-12b"]),
            "[model]"
        );
    }

    #[test]
    fn a_session_override_reaches_the_argv_preview() {
        let mut app = app_with_sample();
        app.llama
            .overrides
            .set(Scope::Model, "gemma4-12b", "ctx-size", "65536");

        let argv = app.llama.argv_preview().expect("preview");
        assert!(argv.windows(2).any(|w| w == ["--ctx-size", "65536"]));
    }

    /// `y` copies the launch the preview is describing, not a rendering of
    /// it: the same argv, the session overrides included, as one runnable
    /// line. The clipboard itself is the Executor's business — `update`
    /// stays pure and only hands over the text.
    /// The preview pane holds six lines; a long argv does not. Scrolling
    /// is bounded at both ends, and the bound is the number of lines the
    /// pane actually hides.
    #[test]
    fn the_argv_preview_scrolls_within_its_bounds() {
        let mut app = app_with_sample();
        app.update(UiEvent::Resize {
            width: 60,
            height: 40,
        });

        let max = app.preview_max_scroll();
        assert!(max > 0, "the fixture argv should not fit in six rows at 60");

        // Held down, it stops at the last line rather than counting on.
        for _ in 0..50 {
            app.update(ch('J'));
        }
        assert_eq!(app.llama.preview_scroll, max);

        // ...so one press of the other key moves immediately.
        app.update(ch('K'));
        assert_eq!(app.llama.preview_scroll, max - 1);

        for _ in 0..50 {
            app.update(ch('K'));
        }
        assert_eq!(app.llama.preview_scroll, 0);
    }

    /// A wider terminal wraps the same argv into fewer lines, so an offset
    /// taken while narrow can point past the end. Resize re-clamps it.
    #[test]
    fn widening_the_terminal_pulls_the_preview_back_into_range() {
        let mut app = app_with_sample();
        app.update(UiEvent::Resize {
            width: 50,
            height: 40,
        });
        for _ in 0..50 {
            app.update(ch('J'));
        }
        assert!(app.llama.preview_scroll > 0);

        app.update(UiEvent::Resize {
            width: 200,
            height: 40,
        });
        assert_eq!(app.preview_max_scroll(), 0, "it all fits at 200 columns");
        assert_eq!(app.llama.preview_scroll, 0, "left scrolled past the end");
    }

    /// Moving to another preset shows another command. Staying scrolled
    /// into the middle of it would be reading the wrong argv.
    #[test]
    fn selecting_another_preset_returns_the_preview_to_the_top() {
        let mut app = app_with_sample();
        app.update(UiEvent::Resize {
            width: 50,
            height: 40,
        });
        app.update(ch('J'));
        assert!(app.llama.preview_scroll > 0);

        app.update(key(KeyCode::Down));
        assert_eq!(app.llama.preview_scroll, 0);
    }

    /// A screen with no preview must not accumulate a scroll offset that
    /// would silently apply when the user comes back to one.
    #[test]
    fn a_screen_without_a_preview_has_nothing_to_scroll() {
        let mut app = app_with_sample();
        jump(&mut app, Screen::Logs);

        assert_eq!(app.preview_max_scroll(), 0);
        app.update(ch('J'));
        assert_eq!(app.llama.preview_scroll, 0);
    }

    #[test]
    fn y_copies_the_launch_command_for_the_highlighted_preset() {
        let mut app = app_with_sample();
        app.llama
            .overrides
            .set(Scope::Model, "gemma4-12b", "ctx-size", "65536");

        let Action::CopyToClipboard { label, text } = app.update(ch('y')) else {
            panic!("y did not copy anything");
        };

        assert_eq!(label, "gemma4-12b");
        assert!(text.starts_with("llama-server "), "{text}");
        assert!(text.contains("--alias gemma4-12b"), "{text}");
        assert!(text.contains("--ctx-size 65536"), "the override: {text}");
        assert!(!text.contains('\n'), "one line, to be pasted: {text}");
        // Not command work: it must stay usable while a launch or a
        // download is in flight.
        assert!(!app.running);
    }

    /// Nothing to copy is said, not silently ignored — a key that
    /// sometimes does nothing without saying so reads as broken.
    #[test]
    fn copying_with_no_config_loaded_says_so() {
        let mut app = App::with_config_path(PathBuf::from("/nonexistent/models.ini"));
        app.screen = Screen::Models;

        assert!(matches!(app.update(ch('y')), Action::None));
        assert!(
            app.logs.iter().any(|line| line.contains("nothing to copy")),
            "{:?}",
            app.logs
        );
    }

    #[test]
    fn settings_rows_cover_server_defaults_and_the_selected_model() {
        let app = app_with_sample();
        let rows = app.llama.setting_rows();

        let headers: Vec<_> = rows
            .iter()
            .filter_map(|row| match row {
                SettingRow::Header(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();

        assert_eq!(
            headers,
            vec![
                "[server]",
                "[*]  defaults",
                "[gemma4-12b]",
                // Last, because it is the last thing applied — and the
                // heading carries the state, since a section that is
                // absent and one that is switched off look identical.
                "[mono-focus]  not in this models.ini",
            ]
        );
        assert!(app.llama.setting_entry_indices().len() >= 7);
    }

    /// The ini with a `[mono-focus]` section, which the two-preset sample
    /// does not carry.
    const PROFILE_INI: &str = r#"
[server]
host = 0.0.0.0
port = 1234

[*]
ctx-size = 32768

[mono-focus]
cache-type-k = q8_0
cache-type-v = q8_0
parallel = 1
cache-reuse = 256
keep = -1

[gemma4-12b]
hf-repo = unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL
"#;

    fn app_with_profile() -> App {
        let path = std::env::temp_dir().join(format!(
            "herd-profile-{}-{:?}.ini",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, PROFILE_INI).expect("write ini");
        App::with_config_path(path)
    }

    /// `m` switches the profile, and the argv preview shows the change —
    /// which is the whole test, since the preview and the launch are the
    /// same call.
    #[test]
    fn m_switches_mono_focus_and_the_argv_follows() {
        let mut app = app_with_profile();
        jump(&mut app, Screen::Settings);

        assert!(!app.llama.mono_focus_on("gemma4-12b"));
        let before = app.llama.argv_preview().expect("argv");
        assert!(!before.iter().any(|a| a == "--cache-reuse"), "{before:?}");

        app.update(ch('m'));

        assert!(app.llama.mono_focus_on("gemma4-12b"));
        let after = app.llama.argv_preview().expect("argv");
        assert!(after.iter().any(|a| a == "--cache-reuse"), "{after:?}");
        assert!(after.iter().any(|a| a == "--keep"), "{after:?}");

        // ...and off again.
        app.update(ch('m'));
        assert!(!app.llama.mono_focus_on("gemma4-12b"));
    }

    /// Its keys are listed only while it is on. Rows on screen that look
    /// editable and are not in force are worse than no rows.
    #[test]
    fn the_profile_keys_appear_only_while_it_is_on() {
        let mut app = app_with_profile();
        let keys = |app: &App| {
            app.llama
                .setting_rows()
                .iter()
                .filter_map(|row| row.as_entry().map(|(_, _, key, _, _)| key.to_string()))
                .filter(|key| key == "cache-reuse")
                .count()
        };

        assert_eq!(keys(&app), 0);
        jump(&mut app, Screen::Settings);
        app.update(ch('m'));
        assert_eq!(keys(&app), 1);
    }

    /// An ini with no such section has no profile to switch on, and says
    /// so rather than setting a flag that changes nothing.
    #[test]
    fn switching_a_profile_that_is_not_in_the_file_says_so() {
        let mut app = app_with_sample();
        jump(&mut app, Screen::Settings);

        app.update(ch('m'));

        assert!(!app.llama.mono_focus_on("gemma4-12b"));
        assert!(app
            .logs
            .back()
            .is_some_and(|line| line.contains("nothing to switch on")));
    }

    /// The toggle is a choice, so it is remembered — with the favourites
    /// and the overrides, not with the session file.
    #[test]
    fn mono_focus_is_carried_into_the_saved_preferences() {
        let mut app = app_with_profile();
        jump(&mut app, Screen::Settings);
        app.update(ch('m'));

        assert!(app.prefs().mono_focus.contains("gemma4-12b"));
    }

    #[test]
    fn editing_a_setting_records_a_session_override() {
        let mut app = app_with_sample();
        jump(&mut app, Screen::Settings);
        app.update(key(KeyCode::Enter)); // edit the first entry ([server] host)

        assert_eq!(app.mode, Mode::EditSetting);
        assert_eq!(app.llama.edit_buffer, "0.0.0.0", "prefilled from the ini");

        for _ in 0.."0.0.0.0".len() {
            app.update(key(KeyCode::Backspace));
        }
        for c in "127.0.0.1".chars() {
            app.update(ch(c));
        }
        app.update(key(KeyCode::Enter));

        assert_eq!(app.mode, Mode::Browse);
        assert_eq!(
            app.llama.overrides.get(Scope::Global, "", "host"),
            Some("127.0.0.1")
        );
    }

    /// Typing `false` over `true` is four keystrokes and a chance to
    /// mistype, for a value with exactly one other possibility. Enter
    /// flips those outright instead of opening the editor.
    #[test]
    fn enter_flips_a_boolean_setting_instead_of_editing_it() {
        let mut app = app_with_sample();
        jump(&mut app, Screen::Settings);

        // Walk to `jinja = true` in [server].
        let jinja = app
            .llama
            .setting_entry_indices()
            .iter()
            .position(|index| {
                matches!(app.llama.setting_rows().get(*index),
                    Some(SettingRow::Entry { key, .. }) if key == "jinja")
            })
            .expect("the sample ini has a jinja key");
        app.llama.settings_cursor = jinja;

        app.update(key(KeyCode::Enter));

        assert_eq!(app.mode, Mode::Browse, "no editor should have opened");
        assert_eq!(
            app.llama.overrides.get(Scope::Global, "", "jinja"),
            Some("false")
        );

        // ...and again, back the other way.
        app.update(key(KeyCode::Enter));
        assert_eq!(
            app.llama.overrides.get(Scope::Global, "", "jinja"),
            Some("true")
        );
    }

    /// The other half of the rule: a value with more than two possibilities
    /// must still open the editor.
    #[test]
    fn enter_still_edits_a_setting_that_is_not_a_boolean() {
        let mut app = app_with_sample();
        jump(&mut app, Screen::Settings);
        app.update(key(KeyCode::Enter)); // [server] host

        assert_eq!(app.mode, Mode::EditSetting);
        assert_eq!(app.llama.edit_buffer, "0.0.0.0");
    }

    /// Retyping the ini's own value should clear the override rather than
    /// pin an identical one that then shows as "modified".
    #[test]
    fn re_entering_the_ini_value_clears_the_override() {
        let mut app = app_with_sample();
        app.llama
            .overrides
            .set(Scope::Global, "", "host", "127.0.0.1");

        jump(&mut app, Screen::Settings);
        app.update(key(KeyCode::Enter));
        for _ in 0.."127.0.0.1".len() {
            app.update(key(KeyCode::Backspace));
        }
        for c in "0.0.0.0".chars() {
            app.update(ch(c));
        }
        app.update(key(KeyCode::Enter));

        assert_eq!(app.llama.overrides.get(Scope::Global, "", "host"), None);
    }

    #[test]
    fn esc_abandons_an_edit_without_recording_anything() {
        let mut app = app_with_sample();
        jump(&mut app, Screen::Settings);
        app.update(key(KeyCode::Enter));
        app.update(ch('9'));
        app.update(key(KeyCode::Esc));

        assert_eq!(app.mode, Mode::Browse);
        assert!(app.llama.overrides.is_empty());
    }

    #[test]
    fn x_clears_one_override_and_shift_x_clears_all() {
        let mut app = app_with_sample();
        app.llama
            .overrides
            .set(Scope::Global, "", "host", "1.2.3.4");
        app.llama.overrides.set(Scope::Global, "", "port", "8080");

        jump(&mut app, Screen::Settings);
        app.update(ch('x')); // cursor is on the first entry: host
        assert_eq!(app.llama.overrides.get(Scope::Global, "", "host"), None);
        assert_eq!(app.llama.overrides.count(), 1);

        app.update(ch('X'));
        assert!(app.llama.overrides.is_empty());
    }

    #[test]
    fn state_transitions_track_the_lifecycle() {
        let mut app = app_with_sample();
        assert_eq!(app.llama.server.state, ServerState::Off);

        app.update(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Starting,
            LauncherMode::Manual,
            Some("gemma4-12b".into()),
        )));
        assert_eq!(app.llama.server.state, ServerState::Starting);
        assert!(app.llama.server.uptime_secs().is_some());
        assert_eq!(
            app.llama.server.endpoint.as_deref(),
            Some("http://127.0.0.1:1234"),
            "0.0.0.0 is rewritten to a connectable host"
        );

        app.update(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Serving,
            LauncherMode::Manual,
            Some("gemma4-12b".into()),
        )));
        assert_eq!(app.llama.server.state, ServerState::Serving);

        app.update(UiEvent::LlamaStatus(LlamaSnapshot::off()));
        assert_eq!(app.llama.server.state, ServerState::Off);
        assert!(app.llama.server.uptime_secs().is_none());
        assert!(app.llama.server.endpoint.is_none());
    }

    #[test]
    fn starting_remembers_the_launched_model() {
        let mut app = app_with_sample();
        app.update(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Starting,
            LauncherMode::Manual,
            Some("qwen3-coder".into()),
        )));
        assert_eq!(app.llama.last_launched.as_deref(), Some("qwen3-coder"));
    }

    #[test]
    fn a_remembered_model_preselects_its_row() {
        let mut app = app_with_sample();
        let path = app.llama.config_path.clone();
        app = App::restored(path, Some("qwen3-coder".into()), Prefs::default());

        assert_eq!(app.llama.cursor, 1);
        assert_eq!(app.llama.selected_model().as_deref(), Some("qwen3-coder"));
    }

    /// Switching tier must report the new path so the Executor follows.
    /// Without this the next launch resolves the preset against the tier
    /// selected at startup and fails with "unknown model".
    #[test]
    fn switching_tier_reports_the_new_config_path() {
        let mut app = app_with_sample();

        let a = std::env::temp_dir().join("herd-tier-a/16gb/models.ini");
        let b = std::env::temp_dir().join("herd-tier-b/32gb/models.ini");
        app.llama.tiers = vec![
            Tier {
                gib: 16,
                name: "16gb".into(),
                config_path: a.clone(),
            },
            Tier {
                gib: 32,
                name: "32gb".into(),
                config_path: b.clone(),
            },
        ];
        app.llama.config_path = a;

        match app.update(ch('t')) {
            Action::ConfigPathChanged(path) => {
                assert_eq!(path, b);
                assert_eq!(app.llama.config_path, b, "App and Action agree");
            }
            other => panic!("expected ConfigPathChanged, got {other:?}"),
        }
    }

    /// With no tiers on this machine there is nothing to switch to, and a
    /// phantom path change must not be announced.
    #[test]
    fn switching_tier_without_tiers_reports_nothing() {
        let mut app = app_with_sample();
        app.llama.tiers.clear();

        assert!(matches!(app.update(ch('t')), Action::None));
    }

    fn serving(app: &mut App, model: &str) {
        app.update(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Serving,
            LauncherMode::Manual,
            Some(model.to_string()),
        )));
    }

    #[test]
    fn the_test_screen_defaults_to_the_scripts_prompt() {
        let app = app_with_sample();
        assert_eq!(app.llama.prompt, llama::api::DEFAULT_PROMPT);
        assert!(app.llama.chat.is_none());
    }

    #[test]
    fn enter_probes_the_serving_model() {
        let mut app = app_with_sample();
        serving(&mut app, "gemma4-12b");
        jump(&mut app, Screen::Test);

        match app.update(key(KeyCode::Enter)) {
            Action::RunChat { model, prompt } => {
                assert_eq!(model, "gemma4-12b");
                assert_eq!(prompt, "Bonjour");
            }
            other => panic!("expected RunChat, got {other:?}"),
        }
        assert!(app.llama.chat_pending);
    }

    /// Probing a server that is not up would just produce a connection
    /// error; say so instead of firing a doomed request.
    #[test]
    fn a_probe_is_refused_while_the_server_is_off() {
        let mut app = app_with_sample();
        jump(&mut app, Screen::Test);

        assert!(matches!(app.update(key(KeyCode::Enter)), Action::None));
        assert!(!app.llama.chat_pending);
    }

    #[test]
    fn a_second_probe_is_refused_while_one_is_in_flight() {
        let mut app = app_with_sample();
        serving(&mut app, "gemma4-12b");
        jump(&mut app, Screen::Test);
        app.update(key(KeyCode::Enter));

        assert!(matches!(app.update(key(KeyCode::Enter)), Action::None));
    }

    #[test]
    fn the_prompt_can_be_edited_and_is_used_for_the_next_probe() {
        let mut app = app_with_sample();
        serving(&mut app, "gemma4-12b");
        jump(&mut app, Screen::Test);
        app.update(ch('e'));

        assert_eq!(app.mode, Mode::EditPrompt);
        assert_eq!(app.llama.edit_buffer, "Bonjour", "prefilled");

        for _ in 0.."Bonjour".len() {
            app.update(key(KeyCode::Backspace));
        }
        for c in "Explique la relativite".chars() {
            app.update(ch(c));
        }
        app.update(key(KeyCode::Enter));

        assert_eq!(app.mode, Mode::Browse);
        assert_eq!(app.llama.prompt, "Explique la relativite");

        match app.update(key(KeyCode::Enter)) {
            Action::RunChat { prompt, .. } => assert_eq!(prompt, "Explique la relativite"),
            other => panic!("expected RunChat, got {other:?}"),
        }
    }

    /// An empty prompt would produce a meaningless probe; keep the old one.
    #[test]
    fn an_empty_prompt_is_rejected() {
        let mut app = app_with_sample();
        jump(&mut app, Screen::Test);
        app.update(ch('e'));
        for _ in 0.."Bonjour".len() {
            app.update(key(KeyCode::Backspace));
        }
        app.update(key(KeyCode::Enter));

        assert_eq!(app.llama.prompt, "Bonjour");
    }

    #[test]
    fn esc_abandons_a_prompt_edit() {
        let mut app = app_with_sample();
        jump(&mut app, Screen::Test);
        app.update(ch('e'));
        app.update(ch('X'));
        app.update(key(KeyCode::Esc));

        assert_eq!(app.mode, Mode::Browse);
        assert_eq!(app.llama.prompt, "Bonjour");
    }

    #[test]
    fn a_chat_result_clears_the_pending_flag_and_is_kept() {
        let mut app = app_with_sample();
        serving(&mut app, "gemma4-12b");
        jump(&mut app, Screen::Test);
        app.update(key(KeyCode::Enter));
        assert!(app.llama.chat_pending);

        // The probe is in flight, so the screen has a clock to show.
        assert!(app.llama.chat_elapsed().is_some());

        app.update(UiEvent::ChatResult(Box::new(Ok(
            llama::api::ChatOutcome::sample(),
        ))));

        assert!(!app.llama.chat_pending);
        assert!(matches!(app.llama.chat, Some(Ok(_))));
        assert!(
            app.llama.chat_elapsed().is_none(),
            "the waiting clock must stop when the result lands"
        );
    }

    #[test]
    fn a_failed_probe_is_reported_and_still_clears_the_flag() {
        let mut app = app_with_sample();
        serving(&mut app, "gemma4-12b");
        jump(&mut app, Screen::Test);
        app.update(key(KeyCode::Enter));

        app.update(UiEvent::ChatResult(Box::new(Err("HTTP 503".into()))));

        assert!(!app.llama.chat_pending);
        assert!(matches!(app.llama.chat, Some(Err(_))));
    }

    /// With nothing loaded, the screen still targets the highlighted
    /// preset so it is usable against a server started outside herd.
    #[test]
    fn the_target_falls_back_to_the_highlighted_preset() {
        let mut app = app_with_sample();
        assert_eq!(app.llama.test_target().as_deref(), Some("gemma4-12b"));

        app.update(key(KeyCode::Down));
        assert_eq!(app.llama.test_target().as_deref(), Some("qwen3-coder"));

        serving(&mut app, "gemma4-12b");
        assert_eq!(
            app.llama.test_target().as_deref(),
            Some("gemma4-12b"),
            "a loaded model wins over the cursor"
        );
    }

    #[test]
    fn c_opens_the_config_picker_on_the_active_entry() {
        let mut app = app_with_sample();
        app.update(ch('c'));

        assert_eq!(app.mode, Mode::Picker);
        let choices = app.llama.config_choices();
        assert_eq!(
            choices[app.llama.picker_cursor].path, app.llama.config_path,
            "the cursor starts on the file in use"
        );
    }

    #[test]
    fn esc_closes_the_picker_without_switching() {
        let mut app = app_with_sample();
        let before = app.llama.config_path.clone();
        app.update(ch('c'));

        assert!(matches!(app.update(key(KeyCode::Esc)), Action::None));
        assert_eq!(app.mode, Mode::Browse);
        assert_eq!(app.llama.config_path, before);
    }

    /// Re-selecting the file already in use is a no-op: nothing to tell
    /// the Executor, and no reason to reset the cursor.
    #[test]
    fn selecting_the_active_config_changes_nothing() {
        let mut app = app_with_sample();
        app.update(ch('c'));

        assert!(matches!(app.update(key(KeyCode::Enter)), Action::None));
        assert_eq!(app.mode, Mode::Browse);
    }

    #[test]
    fn the_picker_lists_the_shipped_tiers_with_their_preset_counts() {
        let mut app = app_with_sample();
        app.llama.ram_gib = Some(36);
        app.llama.tiers = vec![
            Tier {
                gib: 16,
                name: "16gb".into(),
                config_path: shipped("16gb"),
            },
            Tier {
                gib: 32,
                name: "32gb".into(),
                config_path: shipped("32gb"),
            },
        ];

        let choices = app.llama.config_choices();
        let tiers: Vec<_> = choices
            .iter()
            .filter(|choice| choice.label != "current")
            .collect();

        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].presets, 13, "16gb preset count");
        assert_eq!(tiers[1].presets, 9, "32gb preset count");
    }

    /// The warning the picker exists for: on a small machine most of the
    /// 32gb presets do not fit, and the count must say so.
    #[test]
    fn a_small_machine_flags_the_oversized_presets_of_a_tier() {
        let mut app = app_with_sample();
        app.llama.ram_gib = Some(16);
        app.llama.tiers = vec![Tier {
            gib: 32,
            name: "32gb".into(),
            config_path: shipped("32gb"),
        }];

        let tier = app
            .llama
            .config_choices()
            .into_iter()
            .find(|choice| choice.label == "32gb")
            .expect("32gb listed");

        assert!(
            tier.too_large > 0,
            "no 32gb preset flagged on a 16 GiB machine"
        );
        assert!(
            tier.too_large < tier.presets,
            "not every preset is oversized"
        );
    }

    #[test]
    fn a_large_machine_flags_nothing_in_its_own_tier() {
        let mut app = app_with_sample();
        app.llama.ram_gib = Some(36);
        app.llama.tiers = vec![Tier {
            gib: 32,
            name: "32gb".into(),
            config_path: shipped("32gb"),
        }];

        let tier = app
            .llama
            .config_choices()
            .into_iter()
            .find(|choice| choice.label == "32gb")
            .expect("32gb listed");

        assert_eq!(tier.too_large, 0);
    }

    #[test]
    fn selecting_another_config_switches_and_tells_the_executor() {
        let mut app = app_with_sample();
        app.llama.ram_gib = Some(36);
        app.llama.tiers = vec![Tier {
            gib: 32,
            name: "32gb".into(),
            config_path: shipped("32gb"),
        }];

        app.update(ch('c'));
        // The active (sample) file is listed first, the tier after it.
        app.update(key(KeyCode::Down));

        match app.update(key(KeyCode::Enter)) {
            Action::ConfigPathChanged(path) => {
                assert_eq!(path, shipped("32gb"));
                assert_eq!(app.llama.config_path, shipped("32gb"));
                assert_eq!(app.llama.model_names().len(), 9);
            }
            other => panic!("expected ConfigPathChanged, got {other:?}"),
        }
    }

    #[test]
    fn presets_are_sized_and_judged_against_the_budget() {
        let mut app = app_with_sample();
        app.llama.ram_gib = Some(36);

        // The sample's gemma4-12b: ~12B at Q4 plus overhead.
        let estimate = app.llama.estimate_gib("gemma4-12b").expect("estimate");
        assert!((6.0..10.0).contains(&estimate), "estimated {estimate}");
        assert_eq!(app.llama.fit("gemma4-12b"), Fit::Fits);
    }

    /// With no RAM reading there is no budget, so nothing may be claimed
    /// about fit — a red warning built on a guess would be worse than none.
    #[test]
    fn without_a_ram_reading_nothing_is_flagged() {
        let mut app = app_with_sample();
        app.llama.ram_gib = None;

        assert_eq!(app.llama.fit("gemma4-12b"), Fit::Unknown);
    }

    #[test]
    fn the_reserved_ratio_can_be_raised_and_lowered() {
        let mut app = app_with_sample();
        app.llama.ram_gib = Some(32);
        jump(&mut app, Screen::Stats);

        let default = app.llama.reserved_ratio;
        app.update(ch('+'));
        assert!(app.llama.reserved_ratio > default);

        app.update(ch('-'));
        app.update(ch('-'));
        assert!(app.llama.reserved_ratio < default);
    }

    #[test]
    fn the_reserved_ratio_stops_at_its_bounds() {
        let mut app = app_with_sample();
        jump(&mut app, Screen::Stats);

        for _ in 0..40 {
            app.update(ch('-'));
        }
        assert_eq!(app.llama.reserved_ratio, memory::MIN_RESERVED_RATIO);

        for _ in 0..40 {
            app.update(ch('+'));
        }
        assert_eq!(app.llama.reserved_ratio, memory::MAX_RESERVED_RATIO);
    }

    /// Dropping below the system default must be visible, not silent.
    #[test]
    fn lowering_past_the_default_logs_a_caution() {
        let mut app = app_with_sample();
        app.llama.ram_gib = Some(32);
        jump(&mut app, Screen::Stats);
        app.update(ch('-'));

        assert!(app.llama.budget().is_risky());
        assert!(
            app.logs.iter().any(|line| line.contains("CAUTION")),
            "no caution logged: {:?}",
            app.logs
        );
    }

    #[test]
    fn r_restores_the_default_reservation() {
        let mut app = app_with_sample();
        jump(&mut app, Screen::Stats);
        app.update(ch('-'));
        app.update(ch('r'));

        assert_eq!(app.llama.reserved_ratio, memory::DEFAULT_RESERVED_RATIO);
        assert!(!app.llama.budget().is_risky());
    }

    #[test]
    fn f_stars_the_highlighted_preset_and_unstars_it() {
        let mut app = app_with_sample();
        assert!(!app.llama.is_favorite("gemma4-12b"));

        app.update(ch('f'));
        assert!(app.llama.is_favorite("gemma4-12b"));
        assert!(app.logs.iter().any(|line| line.contains("starred")));

        app.update(ch('f'));
        assert!(!app.llama.is_favorite("gemma4-12b"));
    }

    /// The star must not move the row it is on. A table people navigate
    /// by position is not allowed to rearrange itself under them.
    #[test]
    fn starring_a_preset_leaves_the_order_alone() {
        let mut app = app_with_sample();
        let before: Vec<String> = app
            .llama
            .rows()
            .iter()
            .map(|row| row.name.clone())
            .collect();

        app.update(key(KeyCode::Down));
        app.update(ch('f'));

        let after: Vec<String> = app
            .llama
            .rows()
            .iter()
            .map(|row| row.name.clone())
            .collect();
        assert_eq!(before, after);
        assert_eq!(app.llama.cursor, 1, "the cursor followed a reordering");
    }

    /// The whole point of `~/.herd_config`: what was set on purpose is
    /// still set next time. Exercised end to end through the same two
    /// calls `main.rs` makes, since that is where the round trip can
    /// break.
    #[test]
    fn favourites_and_overrides_come_back_next_session() {
        let mut app = app_with_sample();
        app.update(ch('f'));
        app.llama
            .overrides
            .set(Scope::Model, "gemma4-12b", "ctx-size", "65536");
        app.llama.router.models_max = 5;

        let path = app.llama.config_path.clone();
        let saved = app.prefs();

        let restored = App::restored(path, None, saved);
        assert!(restored.llama.is_favorite("gemma4-12b"));
        assert_eq!(
            restored
                .llama
                .overrides
                .get(Scope::Model, "gemma4-12b", "ctx-size"),
            Some("65536")
        );
        assert_eq!(restored.llama.router.models_max, 5);
    }

    #[test]
    fn the_router_screen_adjusts_its_settings_and_stops_at_the_bounds() {
        let mut app = app_with_sample();
        jump(&mut app, Screen::Router);

        app.update(ch('+'));
        assert_eq!(app.llama.router.models_max, prefs::DEFAULT_MODELS_MAX + 1);

        // Held down past the end: it stops, it does not wrap or overflow.
        for _ in 0..40 {
            app.update(ch('-'));
        }
        assert_eq!(app.llama.router.models_max, prefs::MODELS_MAX_RANGE.0);

        app.update(key(KeyCode::Down));
        app.update(ch('+'));
        assert_eq!(
            app.llama.router.sleep_idle_seconds,
            prefs::DEFAULT_SLEEP_IDLE_SECONDS + prefs::SLEEP_IDLE_STEP
        );

        app.update(ch('r'));
        assert_eq!(app.llama.router, RouterPrefs::default());
    }

    /// Enter starts the router with the numbers on screen — the reason
    /// the screen exists, and the one thing that must not go through a
    /// re-typed command line.
    #[test]
    fn enter_on_the_router_screen_starts_it_with_the_settings_shown() {
        let mut app = app_with_sample();
        jump(&mut app, Screen::Router);
        app.llama.router = RouterPrefs {
            models_max: 4,
            sleep_idle_seconds: 120,
        };

        match app.update(key(KeyCode::Enter)) {
            Action::RunCommand(command) => assert_eq!(command, "router --max 4 --idle 120"),
            other => panic!("expected a router command, got {other:?}"),
        }
    }

    #[test]
    fn the_router_preview_is_the_argv_the_router_would_spawn() {
        let mut app = app_with_sample();
        app.llama.router = RouterPrefs {
            models_max: 3,
            sleep_idle_seconds: 60,
        };

        let argv = app.llama.router_argv_preview().expect("preview");
        assert!(
            argv.windows(2).any(|w| w == ["--models-max", "3"]),
            "{argv:?}"
        );
        assert!(
            argv.windows(2).any(|w| w == ["--sleep-idle-seconds", "60"]),
            "{argv:?}"
        );
        assert!(
            argv.iter().any(|token| token == "--models-preset"),
            "the router serves the whole file: {argv:?}"
        );
        // ...and it is the same [server] block the presets inherit.
        assert!(argv.windows(2).any(|w| w == ["--port", "1234"]), "{argv:?}");
    }

    /// Freeing reserved memory is the point of the override: it must
    /// actually move a preset from "too large" to launchable.
    #[test]
    fn lowering_the_reservation_can_bring_a_preset_into_range() {
        let mut app = app_with_sample();
        app.llama.ram_gib = Some(10);
        app.llama.reserved_ratio = 0.60;
        assert_eq!(app.llama.fit("gemma4-12b"), Fit::TooLarge);

        app.llama.reserved_ratio = memory::MIN_RESERVED_RATIO;
        assert_ne!(app.llama.fit("gemma4-12b"), Fit::TooLarge);
    }

    #[test]
    fn stats_reset_when_a_new_model_starts() {
        let mut app = app_with_sample();
        app.llama.stats.probes = 7;
        app.llama.stats.completion_tokens = 500;

        app.update(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Starting,
            LauncherMode::Manual,
            Some("gemma4-12b".into()),
        )));

        assert_eq!(app.llama.stats.probes, 0);
        assert_eq!(app.llama.stats.completion_tokens, 0);
        assert!(app.llama.stats.started_at.is_some(), "start time recorded");
    }

    #[test]
    fn stop_does_nothing_when_the_server_is_already_off() {
        let mut app = app_with_sample();
        assert!(matches!(app.update(ch('s')), Action::None));
        assert!(!app.running);
    }

    #[test]
    fn stop_is_queued_while_the_server_is_live() {
        let mut app = app_with_sample();
        app.llama.server.state = ServerState::Serving;

        match app.update(ch('s')) {
            Action::RunCommand(command) => assert_eq!(command, "stop"),
            other => panic!("expected RunCommand, got {other:?}"),
        }
    }

    /// Every feature that talks to the server has to say so when there is
    /// none, rather than failing in the plumbing's terms or — worse —
    /// doing nothing at all.
    ///
    /// `p` on the Server screen was the silent case: with nothing running
    /// there is no model name to ping, and the key returned `None` without
    /// a word, which is indistinguishable from a key that is not bound.
    /// The Test screen's Enter already refused out loud, and `:status` and
    /// `:ping` now explain themselves from `api::unreachable`.
    #[test]
    fn pinging_with_no_server_says_so_rather_than_nothing() {
        let mut app = app_with_sample();
        app.screen = Screen::Server;
        assert!(app.llama.server.model.is_none());

        let logs = app.logs.len();
        assert!(matches!(app.update(ch('p')), Action::None));
        assert!(app.logs.len() > logs, "the key did nothing at all");
        assert!(app
            .logs
            .back()
            .is_some_and(|line| line.contains("no llama-server is running")));
    }

    /// The Test screen's own guard, for the same question asked a
    /// different way.
    #[test]
    fn a_chat_probe_with_no_server_refuses_before_the_network() {
        let mut app = app_with_sample();
        jump(&mut app, Screen::Test);

        let logs = app.logs.len();
        assert!(matches!(app.update(key(KeyCode::Enter)), Action::None));
        assert!(!app.llama.chat_pending, "a probe was dispatched anyway");
        assert!(app.logs.len() > logs, "the key did nothing at all");
    }

    /// Stop is the command you reach for when something else is stuck, so
    /// gating it on nothing else being in flight had it refuse exactly when
    /// it was needed: a launch that was slow to spawn — or slow to be
    /// killed — held `running`, and every stop keypress was answered with
    /// "busy, ignored :stop".
    #[test]
    fn stop_is_dispatched_even_while_another_command_is_in_flight() {
        let mut app = app_with_sample();
        app.llama.server.state = ServerState::Starting;
        app.running = true;

        match app.update(ch('s')) {
            Action::RunCommand(command) => assert_eq!(command, "stop"),
            other => panic!("stop was swallowed by the busy gate: {other:?}"),
        }
    }

    /// Types a whole command line into the bar and submits it.
    fn submit(app: &mut App, line: &str) -> Action {
        app.update(ch(':'));
        for c in line.chars() {
            app.update(ch(c));
        }
        app.update(key(KeyCode::Enter))
    }

    /// `:help` answers on the spot, as an overlay over whatever is on
    /// screen. It used to be dispatched like any other command and printed
    /// into the log — which is on another screen, is where a loading
    /// server also writes hundreds of lines, and scrolls.
    #[test]
    fn help_opens_the_command_listing_rather_than_running_anything() {
        let mut app = app_with_sample();

        assert!(matches!(submit(&mut app, "help"), Action::None));
        assert_eq!(app.mode, Mode::Commands);
        assert!(!app.running, "the listing is local, nothing was queued");
        assert!(app.command_input.is_empty());

        // A reference card closes on any key, like the `?` overlay.
        app.update(ch('x'));
        assert_eq!(app.mode, Mode::Browse);
    }

    /// "What can I type" and "what am I running" are questions people ask
    /// *because* something is stuck, so the busy gate must not be what
    /// answers them.
    #[test]
    fn the_local_overlays_are_answered_while_a_command_is_in_flight() {
        for (typed, expected) in [("help", Mode::Commands), ("about", Mode::About)] {
            let mut app = app_with_sample();
            app.running = true;

            assert!(matches!(submit(&mut app, typed), Action::None), "{typed}");
            assert_eq!(app.mode, expected, "{typed}");
        }
    }

    /// `:about` answers on the spot too, and closes on any key.
    #[test]
    fn about_opens_the_build_details_rather_than_running_anything() {
        let mut app = app_with_sample();

        assert!(matches!(submit(&mut app, "about"), Action::None));
        assert_eq!(app.mode, Mode::About);
        assert!(!app.running, "the dialog is local, nothing was queued");
        assert!(app.command_input.is_empty());

        app.update(ch('x'));
        assert_eq!(app.mode, Mode::Browse);
    }

    /// The other half of `every_documented_command_reaches_a_handler`
    /// (in `executor.rs`): the commands answered here, in `App`, must be
    /// answered here — reaching the Executor would mean falling through to
    /// the generic script path and coming back "Unknown command".
    #[test]
    fn every_locally_handled_command_is_answered_by_the_app() {
        use crate::commands::{Handler, ALL};

        for command in ALL.iter().filter(|c| c.handler == Handler::App) {
            let mut app = app_with_sample();
            let logs = app.logs.len();

            match submit(&mut app, command.probe) {
                Action::None => {}
                other => panic!("`:{}` was dispatched instead: {other:?}", command.usage),
            }

            // "Visible" is either an overlay or a log line — the two ways
            // a local command can answer. Named that way rather than as
            // one specific mode, so a command that opens a *different*
            // overlay still counts.
            assert!(
                app.mode != Mode::Browse || app.logs.len() > logs,
                "`:{}` did nothing visible at all",
                command.usage
            );
            assert!(
                !app.running,
                "`:{}` claimed the busy flag for local work",
                command.usage
            );
        }
    }

    /// The same for a typed `:stop`, which reaches the dispatcher by a
    /// different route and would otherwise be dropped by the busy check.
    #[test]
    fn a_typed_stop_is_dispatched_even_while_busy() {
        let mut app = app_with_sample();
        app.running = true;
        app.update(ch(':'));
        for c in "stop".chars() {
            app.update(ch(c));
        }

        match app.update(key(KeyCode::Enter)) {
            Action::RunCommand(command) => assert_eq!(command, "stop"),
            other => panic!("typed :stop was swallowed by the busy gate: {other:?}"),
        }
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.command_input.is_empty());
    }

    /// Dispatching a stop must not claim the `running` flag: it would then
    /// clear on its own completion and un-gate whatever command it
    /// overlapped with.
    #[test]
    fn stop_does_not_claim_the_busy_flag() {
        let mut app = app_with_sample();
        app.llama.server.state = ServerState::Serving;

        app.update(ch('s'));
        assert!(!app.running);
    }

    /// At rest nothing on screen advances, so an idle tick has nothing to
    /// redraw for. Getting this wrong in the other direction is the real
    /// risk — a clock that stops ticking — so both halves are pinned.
    #[test]
    fn only_a_running_clock_makes_an_idle_tick_worth_drawing() {
        let mut app = app_with_sample();
        assert!(!app.llama.ticking(), "an idle app redraws for nothing");

        // A launch starts the uptime clock.
        app.update(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Starting,
            LauncherMode::Manual,
            Some("gemma4-12b".into()),
        )));
        assert!(app.llama.ticking(), "the elapsed counter would freeze");

        // Stopping puts it away again.
        app.update(UiEvent::LlamaStatus(LlamaSnapshot::off()));
        assert!(!app.llama.ticking());

        // ...and so does a probe in flight, which counts up while waiting.
        app.llama.chat_pending = true;
        assert!(app.llama.ticking(), "the waiting counter would freeze");
    }

    /// A failed launch left ERROR on screen with no key that cleared it:
    /// `s` did nothing, because there was nothing running to stop. The
    /// state outlived the failure it described.
    #[test]
    fn stop_clears_a_stale_error() {
        let mut app = app_with_sample();
        app.update(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Error("download stalled at 4.5G".into()),
            LauncherMode::Manual,
            Some("gemma4-31b".into()),
        )));
        assert!(matches!(app.llama.server.state, ServerState::Error(_)));

        assert!(matches!(app.update(ch('s')), Action::None));
        assert_eq!(app.llama.server.state, ServerState::Off);
        assert_eq!(app.llama.server.model, None);

        let logs = app.logs.iter().cloned().collect::<Vec<_>>().join("\n");
        assert!(logs.contains("download stalled"), "{logs}");
    }

    /// ...but stopping a live server is still a real stop, not a local
    /// state edit.
    #[test]
    fn stop_on_a_live_server_still_dispatches() {
        let mut app = app_with_sample();
        app.llama.server.state = ServerState::Serving;

        assert!(matches!(app.update(ch('s')), Action::RunCommand(_)));
        assert_eq!(app.llama.server.state, ServerState::Serving, "not local");
    }

    /// Relaunching what is already up is a stop and a full reload for no
    /// gain — minutes of it, on a machine where loading is slow.
    #[test]
    fn enter_on_the_server_screen_refuses_to_relaunch_what_is_serving() {
        let mut app = app_with_sample();
        app.screen = Screen::Server;
        app.update(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Serving,
            LauncherMode::Manual,
            app.llama.selected_model(),
        )));

        assert!(app.llama.relaunch_blocked().is_some());
        assert!(matches!(app.update(key(KeyCode::Enter)), Action::None));

        let logs = app.logs.iter().cloned().collect::<Vec<_>>().join("\n");
        assert!(logs.contains("already serving"), "{logs}");
    }

    /// ...but a *different* preset is a legitimate hot-swap, and must
    /// still go through.
    #[test]
    fn enter_on_the_server_screen_still_swaps_to_another_preset() {
        let mut app = app_with_sample();
        app.screen = Screen::Server;
        app.update(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Serving,
            LauncherMode::Manual,
            Some("some-other-model".into()),
        )));
        app.update(UiEvent::CacheList(vec![
            "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL".into(),
        ]));

        assert!(app.llama.relaunch_blocked().is_none());
        match app.update(key(KeyCode::Enter)) {
            Action::RunCommand(command) => assert_eq!(command, "launch gemma4-12b"),
            other => panic!("expected a launch, got {other:?}"),
        }
    }

    /// The Models screen deliberately keeps the relaunch: pressing Enter
    /// there after changing a setting is how a session override is
    /// applied, which is a real workflow the guard must not break.
    #[test]
    fn the_models_screen_still_relaunches_the_serving_preset() {
        let mut app = app_with_sample();
        app.update(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Serving,
            LauncherMode::Manual,
            app.llama.selected_model(),
        )));
        app.update(UiEvent::CacheList(vec![
            "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL".into(),
        ]));

        match app.update(key(KeyCode::Enter)) {
            Action::RunCommand(command) => assert_eq!(command, "launch gemma4-12b"),
            other => panic!("relaunch from Models was blocked: {other:?}"),
        }
    }

    /// Nothing is running, so there is nothing to protect.
    #[test]
    fn nothing_is_blocked_while_the_server_is_off() {
        let app = app_with_sample();
        assert!(app.llama.relaunch_blocked().is_none());
    }

    /// With no supervised process there is nothing for HERD to stop.
    #[test]
    fn quitting_with_nothing_running_does_not_ask() {
        let mut app = app_with_sample();

        assert!(app.in_flight().is_empty());
        assert!(matches!(app.update(ch('q')), Action::Quit));
        assert_eq!(app.mode, Mode::Browse);
    }

    /// A supervised server is deliberately *not* work in flight: stopping
    /// it on exit is the documented behaviour, every time.
    #[test]
    fn a_running_server_asks_but_is_not_listed_as_abandoned_work() {
        let mut app = app_with_sample();
        app.llama.server.state = ServerState::Serving;
        app.llama.server.mode = LauncherMode::Manual;

        assert!(app.in_flight().is_empty());
        assert!(matches!(app.update(ch('q')), Action::None));
        assert_eq!(app.mode, Mode::ConfirmQuit);
    }

    #[test]
    fn a_live_router_asks_before_quitting() {
        let mut app = app_with_sample();
        app.llama.server.state = ServerState::Starting;
        app.llama.server.mode = LauncherMode::Router;

        assert!(matches!(app.update(ch('q')), Action::None));
        assert_eq!(app.mode, Mode::ConfirmQuit);
    }

    #[test]
    fn a_failed_server_does_not_trigger_quit_confirmation() {
        let mut app = app_with_sample();
        app.llama.server.state = ServerState::Error("failed".into());
        app.llama.server.mode = LauncherMode::Manual;

        assert!(matches!(app.update(ch('q')), Action::Quit));
        assert_eq!(app.mode, Mode::Browse);
    }

    #[test]
    fn a_download_without_a_live_server_does_not_trigger_the_server_prompt() {
        let mut app = app_with_sample();
        app.update(UiEvent::DownloadProgress {
            model: "gemma4-12b".into(),
            done: 1_000,
            total: 4_000,
        });

        assert!(matches!(app.update(ch('q')), Action::Quit));
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.in_flight()[0].contains("gemma4-12b"));
    }

    #[test]
    fn declining_the_quit_prompt_stays_put() {
        let mut app = app_with_sample();
        app.llama.chat_pending = true;
        app.llama.server.state = ServerState::Serving;
        app.llama.server.mode = LauncherMode::Manual;

        app.update(ch('q'));
        assert_eq!(app.mode, Mode::ConfirmQuit);

        assert!(matches!(app.update(ch('n')), Action::None));
        assert_eq!(app.mode, Mode::Browse);
    }

    /// The force variant, in the same spirit as `launch!`: the answer
    /// given before the question is asked.
    #[test]
    fn shift_q_quits_without_asking() {
        let mut app = app_with_sample();
        app.llama.chat_pending = true;
        app.running = true;

        assert!(matches!(app.update(ch('Q')), Action::Quit));
        assert_eq!(app.mode, Mode::Browse, "no prompt should have opened");
    }

    /// Everything that would be abandoned is named, so the prompt says
    /// what is at stake rather than just "are you sure".
    #[test]
    fn in_flight_names_every_kind_of_work() {
        let mut app = app_with_sample();
        app.running = true;
        app.llama.chat_pending = true;
        app.llama.download = Some(Download {
            model: "qwen3-14b".into(),
            done: 1,
            total: 2,
        });

        let work = app.in_flight();
        assert_eq!(work.len(), 3, "{work:?}");
        assert!(work.iter().any(|w| w.contains("qwen3-14b")));
        assert!(work.iter().any(|w| w.contains("test")));
        assert!(work.iter().any(|w| w.contains("command")));
    }

    /// The columns describe what the ini and the repo name actually say.
    #[test]
    fn a_preset_reports_its_optimisations_and_capabilities() {
        let app = App::with_config_path(shipped("32gb"));

        // gemma4-12b: qat + dynamic quant, vision turned off, MTP in use.
        let opts = app.llama.optimisations("gemma4-12b");
        assert!(opts.contains(&llama::caps::Optimisation::Qat));
        assert!(opts.contains(&llama::caps::Optimisation::Dynamic));

        // spec-type = draft-mtp, so speculative decoding is actually on.
        let traits = app.llama.capabilities("gemma4-12b");
        assert!(traits
            .iter()
            .any(|t| t.capability == llama::caps::Capability::Speculative && t.enabled));

        // ...and no vision, despite `no-mmproj = true`: the flag is set
        // defensively across these tiers and is not evidence of a projector.
        assert!(traits
            .iter()
            .all(|t| t.capability != llama::caps::Capability::Vision));
    }

    /// A preset with no repo cannot be described, and must not invent
    /// anything to fill the columns.
    #[test]
    fn a_preset_without_a_repo_reports_nothing() {
        let app = App::with_config_path(PathBuf::from("/nonexistent/models.ini"));

        assert!(app.llama.optimisations("whatever").is_empty());
        assert!(app.llama.capabilities("whatever").is_empty());
    }

    /// A cache entry with a size on it, as `--cache-list` plus the
    /// measuring pass produces.
    fn measured(reference: &str, weights: u64, disk: u64) -> llama::hub::CachedModel {
        llama::hub::CachedModel {
            weights: Some(weights),
            bytes: Some(disk),
            ..llama::hub::CachedModel::from(reference)
        }
    }

    /// The size a preset shows is the file on disk once there is one.
    ///
    /// The heuristic is documented as approximate and has been wrong by a
    /// factor of four; the weights are right there and can simply be read.
    /// The `~` is what says which of the two a row is showing.
    #[test]
    fn a_downloaded_preset_is_measured_rather_than_estimated() {
        let mut app = app_with_sample();

        let estimated = app.llama.sizing("gemma4-12b").expect("a sizeable preset");
        assert!(!estimated.is_measured());
        assert!(estimated.label().starts_with('~'), "{}", estimated.label());

        // 8 GiB of weights, in a repo whose disk holds a stale revision too.
        app.update(UiEvent::CacheList(vec![measured(
            "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL",
            8 * 1024 * 1024 * 1024,
            20 * 1024 * 1024 * 1024,
        )]));

        let sizing = app.llama.sizing("gemma4-12b").expect("measured");
        assert!(sizing.is_measured());
        // The weights plus the same runtime allowance the estimate adds,
        // so a measured row and an estimated one mean the same thing.
        assert_eq!(sizing.label(), "9.0G");
        // ...and the disk usage of the repo is emphatically *not* it.
        assert!(sizing.gib() < 12.0, "{}", sizing.gib());
    }

    /// The Hub screen lists the cache and says what this tier makes of it.
    #[test]
    fn the_hub_lists_the_cache_and_names_the_presets_that_use_it() {
        let mut app = app_with_sample();
        assert!(
            app.llama.hub_rows().is_empty(),
            "the cache was listed before llama.cpp answered"
        );

        app.update(UiEvent::CacheList(vec![
            measured("unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL", 8_000, 20_000),
            measured("vendor/Nobody-Asked-For-This-GGUF:Q4_K_M", 3_000, 3_000),
        ]));

        let rows = app.llama.hub_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].preset.as_deref(), Some("gemma4-12b"));
        assert_eq!(
            rows[1].preset, None,
            "an unreferenced model claimed a preset"
        );
        assert!(!rows[0].shares_disk);
        // Counted once per repo, so two entries of one repo cannot double
        // the total.
        assert_eq!(app.llama.hub_disk_bytes(), 23_000);
    }

    /// Two quantisations of one repo share a blobs directory, and the
    /// screen has to say so rather than adding the same disk up twice.
    #[test]
    fn two_quantisations_of_one_repo_share_their_disk() {
        let mut app = app_with_sample();
        app.update(UiEvent::CacheList(vec![
            measured("unsloth/Qwen3-14B-GGUF:Q4_K_XL", 8_000, 20_000),
            measured("unsloth/Qwen3-14B-GGUF:Q8_0", 12_000, 20_000),
        ]));

        assert!(app.llama.hub_rows().iter().all(|row| row.shares_disk));
        assert_eq!(
            app.llama.hub_disk_bytes(),
            20_000,
            "the repo was counted twice"
        );
    }

    /// The gap between "on this machine" and "in this tier" is three lines
    /// of ini, and copying them beats retyping a repo reference off a
    /// terminal — which is where the typo that fetches a second copy comes
    /// from.
    #[test]
    fn y_on_the_hub_copies_a_preset_stanza() {
        let mut app = app_with_sample();
        jump(&mut app, Screen::Hub);
        app.update(UiEvent::CacheList(vec![measured(
            "unsloth/Qwen3-14B-GGUF:Q4_K_XL",
            8_000,
            8_000,
        )]));

        match app.update(ch('y')) {
            Action::CopyToClipboard { text, .. } => {
                assert!(text.contains("[qwen3-14b]"), "{text}");
                assert!(
                    text.contains("hf-repo = unsloth/Qwen3-14B-GGUF:Q4_K_XL"),
                    "{text}"
                );
                assert!(text.contains("alias = qwen3-14b"), "{text}");
            }
            other => panic!("expected a clipboard copy, got {other:?}"),
        }
    }

    /// Enter jumps to the preset that names the highlighted model, since
    /// everything that acts on a preset lives on the Models screen.
    #[test]
    fn enter_on_the_hub_reveals_the_preset_on_the_models_screen() {
        let mut app = app_with_sample();
        app.update(UiEvent::CacheList(vec![
            measured("vendor/unreferenced-GGUF:Q4_K_M", 1, 1),
            measured("unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_XL", 1, 1),
        ]));
        jump(&mut app, Screen::Hub);

        // The first row is in no tier: there is nothing to reveal, and the
        // cursor must not move somewhere arbitrary instead.
        app.update(key(KeyCode::Enter));
        assert_eq!(app.screen, Screen::Hub);

        app.update(ch('j'));
        app.update(key(KeyCode::Enter));
        assert_eq!(app.screen, Screen::Models);
        assert_eq!(app.llama.selected_model().as_deref(), Some("qwen3-coder"));
    }

    /// `D` asks before it deletes, and the prompt carries what the modal
    /// has to say: the repo, the size, and anything sharing the directory.
    #[test]
    fn shift_d_on_the_hub_asks_before_deleting() {
        let mut app = app_with_sample();
        app.update(UiEvent::CacheList(vec![
            measured("unsloth/Qwen3-14B-GGUF:Q4_K_XL", 8_000, 20_000),
            measured("unsloth/Qwen3-14B-GGUF:Q8_0", 12_000, 20_000),
        ]));
        jump(&mut app, Screen::Hub);

        assert!(matches!(app.update(ch('D')), Action::None));
        assert_eq!(app.mode, Mode::ConfirmDelete);

        let pending = app.llama.pending_delete.clone().expect("a pending delete");
        assert_eq!(pending.repo, "unsloth/Qwen3-14B-GGUF");
        assert_eq!(pending.bytes, Some(20_000));
        assert_eq!(
            pending.also_removes,
            vec!["unsloth/Qwen3-14B-GGUF:Q8_0".to_string()],
            "the other quantisation in the same directory is not named"
        );
    }

    /// Confirming dispatches the deletion; anything else is a cancel.
    #[test]
    fn only_a_lowercase_y_confirms_a_deletion() {
        let mut app = app_with_sample();
        app.update(UiEvent::CacheList(vec![measured(
            "unsloth/Qwen3-14B-GGUF:Q4_K_XL",
            8_000,
            8_000,
        )]));
        jump(&mut app, Screen::Hub);

        // Anything else cancels — including the capital Y the launch
        // prompts accept, since a slipped shift key must not be what
        // destroys a download.
        for cancel in ['Y', 'n', 'd'] {
            app.update(ch('D'));
            assert!(matches!(app.update(ch(cancel)), Action::None), "{cancel}");
            assert_eq!(app.mode, Mode::Browse);
            assert!(app.llama.pending_delete.is_none());
        }

        app.update(ch('D'));
        match app.update(ch('y')) {
            Action::DeleteModel { reference, repo } => {
                assert_eq!(reference, "unsloth/Qwen3-14B-GGUF:Q4_K_XL");
                assert_eq!(repo, "unsloth/Qwen3-14B-GGUF");
            }
            other => panic!("expected a deletion, got {other:?}"),
        }
    }

    /// Pulling the weights out from under a running server has no sensible
    /// "yes anyway", so it is refused rather than confirmed.
    #[test]
    fn a_serving_model_is_not_offered_for_deletion() {
        let mut app = app_with_sample();
        app.update(UiEvent::CacheList(vec![measured(
            "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL",
            8_000,
            8_000,
        )]));
        app.update(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Serving,
            LauncherMode::Manual,
            Some("gemma4-12b".into()),
        )));
        jump(&mut app, Screen::Hub);

        assert!(matches!(app.update(ch('D')), Action::None));
        assert_eq!(app.mode, Mode::Browse, "it asked anyway");
        assert!(app.llama.pending_delete.is_none());
        assert!(
            app.logs.iter().any(|line| line.contains("stop it before")),
            "no reason was given"
        );
    }

    /// The same for a directory something is still writing into.
    #[test]
    fn a_model_being_downloaded_is_not_offered_for_deletion() {
        let mut app = app_with_sample();
        app.update(UiEvent::CacheList(vec![measured(
            "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL",
            8_000,
            8_000,
        )]));
        app.update(UiEvent::DownloadProgress {
            model: "gemma4-12b".into(),
            done: 1,
            total: 8_000,
        });
        jump(&mut app, Screen::Hub);

        assert!(matches!(app.update(ch('D')), Action::None));
        assert_eq!(app.mode, Mode::Browse);
        assert!(app
            .logs
            .iter()
            .any(|line| line.contains("wait for it to finish")));
    }

    /// A refresh that finds fewer models must not leave the cursor past
    /// the end of the list.
    #[test]
    fn the_hub_cursor_survives_a_shorter_cache() {
        let mut app = app_with_sample();
        app.update(UiEvent::CacheList(vec![
            measured("a/one-GGUF:Q4_K_M", 1, 1),
            measured("a/two-GGUF:Q4_K_M", 1, 1),
        ]));
        jump(&mut app, Screen::Hub);
        app.update(ch('G'));
        assert_eq!(app.llama.hub_cursor, 1);

        app.update(UiEvent::CacheList(vec![measured(
            "a/one-GGUF:Q4_K_M",
            1,
            1,
        )]));
        assert_eq!(app.llama.hub_cursor, 0);
        assert!(app.llama.selected_hub().is_some());
    }

    /// Until llama.cpp has been asked, nothing is claimed. Telling someone
    /// to download a model they already have is the one mistake this
    /// feature can make, so it defaults to silence.
    #[test]
    fn availability_is_unknown_before_the_cache_has_been_read() {
        let app = app_with_sample();

        assert_eq!(app.llama.cached, None);
        assert_eq!(app.llama.availability("gemma4-12b"), Availability::Unknown);
        assert_eq!(app.llama.availability("gemma4-12b").label(), None);
    }

    #[test]
    fn the_cache_list_decides_what_is_local() {
        let mut app = app_with_sample();
        app.update(UiEvent::CacheList(vec![
            "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL".into(),
        ]));

        assert_eq!(app.llama.availability("gemma4-12b"), Availability::Local);

        app.update(UiEvent::CacheList(vec![]));
        assert_eq!(app.llama.availability("gemma4-12b"), Availability::Missing);
        assert_eq!(
            app.llama.availability("gemma4-12b").label(),
            Some("not local")
        );
    }

    /// Several gigabytes over a domestic connection is not something to
    /// start because someone pressed Enter on a row.
    #[test]
    fn launching_a_missing_preset_asks_first() {
        let mut app = app_with_sample();
        app.update(UiEvent::CacheList(vec![]));

        assert!(matches!(app.update(key(KeyCode::Enter)), Action::None));
        assert_eq!(app.mode, Mode::ConfirmLaunch);
        assert!(matches!(
            app.llama.confirm,
            Some(Confirm::NotDownloaded { .. })
        ));
    }

    /// Confirming downloads *and then* launches: the user asked for the
    /// model, not for a download.
    #[test]
    fn confirming_a_missing_preset_downloads_then_launches() {
        let mut app = app_with_sample();
        app.update(UiEvent::CacheList(vec![]));
        app.update(key(KeyCode::Enter));

        match app.update(ch('y')) {
            Action::Download {
                model,
                repo,
                then_launch,
                ..
            } => {
                assert_eq!(model, "gemma4-12b");
                assert!(repo.starts_with("unsloth/gemma-4-12B"));
                assert!(then_launch, "the launch the user asked for was dropped");
            }
            other => panic!("expected a download, got {other:?}"),
        }
    }

    /// `d` fetches without launching, so a mostly-empty tier can be filled
    /// ahead of time rather than one model at a time on demand.
    #[test]
    fn d_downloads_without_launching() {
        let mut app = app_with_sample();
        app.update(UiEvent::CacheList(vec![]));

        match app.update(ch('d')) {
            Action::Download { then_launch, .. } => {
                assert!(!then_launch, "d must not start the server")
            }
            other => panic!("expected a download, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_hf_disables_downloads_without_disabling_the_app() {
        let mut app = app_with_sample();
        app.tools.hf.version = Err("`hf` was not found on PATH".into());
        app.update(UiEvent::CacheList(vec![]));

        assert!(matches!(app.update(ch('d')), Action::None));
        assert!(app.llama.download.is_none());
        assert!(app
            .logs
            .back()
            .is_some_and(|line| line.contains("cannot download gemma4-12b")));
    }

    #[test]
    fn d_on_an_already_local_preset_does_nothing() {
        let mut app = app_with_sample();
        app.update(UiEvent::CacheList(vec![
            "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL".into(),
        ]));

        assert!(matches!(app.update(ch('d')), Action::None));
        assert!(app.llama.download.is_none());
    }

    /// Two `hf` processes writing into the same cache is a fight nobody
    /// needs, and the screen has one bar.
    #[test]
    fn only_one_download_runs_at_a_time() {
        let mut app = app_with_sample();
        app.update(UiEvent::CacheList(vec![]));
        app.update(ch('d'));

        assert!(matches!(app.update(ch('d')), Action::None));
    }

    #[test]
    fn download_progress_and_completion_drive_the_bar() {
        let mut app = app_with_sample();

        app.update(UiEvent::DownloadProgress {
            model: "gemma4-12b".into(),
            done: 2_000,
            total: 8_000,
        });
        let bar = app.llama.download.clone().expect("a bar to draw");
        assert_eq!(bar.ratio(), 0.25);
        assert!(bar.label().contains("25%"), "{}", bar.label());

        app.update(UiEvent::DownloadFinished {
            model: "gemma4-12b".into(),
            result: Box::new(Ok("done".into())),
        });
        assert!(
            app.llama.download.is_none(),
            "the bar outlived the download"
        );
    }

    /// A total of zero is "we have not asked yet", not "nothing to do".
    /// Dividing by it would be a panic or a NaN width.
    #[test]
    fn a_download_of_unknown_size_does_not_divide_by_zero() {
        let unknown = Download {
            model: "m".into(),
            done: 0,
            total: 0,
        };
        assert_eq!(unknown.ratio(), 0.0);
    }

    /// The extra artifacts are read from the preset, not assumed: fetching
    /// a vision projector for a preset that says `no-mmproj` is a few
    /// hundred megabytes nobody asked for.
    #[test]
    fn the_extra_artifacts_come_from_the_preset() {
        let app = App::with_config_path(shipped("32gb"));

        let names = app.llama.model_names();
        let mtp = names
            .iter()
            .find(|name| {
                app.llama
                    .config
                    .as_ref()
                    .and_then(|c| effective(c, name, &["spec-type"]))
                    .is_some_and(|spec| spec.contains("mtp"))
            })
            .expect("the 32gb tier has a draft-mtp preset");

        assert!(app.llama.wants(mtp).mtp, "an MTP preset needs its MTP head");

        let no_mmproj = names
            .iter()
            .find(|name| {
                app.llama
                    .config
                    .as_ref()
                    .and_then(|c| effective(c, name, &["no-mmproj"]))
                    .is_some()
            })
            .expect("the 32gb tier has a no-mmproj preset");

        assert!(
            !app.llama.wants(no_mmproj).mmproj,
            "no-mmproj must not pull the projector"
        );
    }

    /// The "TOO LARGE" marker on the Models screen was decoration until
    /// now. On a machine with no headroom, launching anyway is the fastest
    /// way to wedge the whole desktop, so it is worth one keystroke.
    #[test]
    fn an_oversized_preset_asks_before_launching() {
        let mut app = app_with_sample();
        app.llama.ram_gib = Some(10);
        app.llama.reserved_ratio = 0.60;
        assert_eq!(app.llama.fit("gemma4-12b"), Fit::TooLarge);

        assert!(matches!(app.update(key(KeyCode::Enter)), Action::None));
        assert_eq!(app.mode, Mode::ConfirmLaunch);
        assert!(matches!(app.llama.confirm, Some(Confirm::TooLarge { .. })));
        assert!(!app.running, "nothing may run before the answer");
    }

    /// Confirming the memory warning launches normally — *not* with the
    /// `launch!` force variant. The user accepted the memory risk, not a
    /// busy port, and must still be asked if the port turns out to be taken.
    #[test]
    fn confirming_an_oversized_launch_still_checks_the_port() {
        let mut app = app_with_sample();
        app.llama.ram_gib = Some(10);
        app.llama.reserved_ratio = 0.60;
        app.update(key(KeyCode::Enter));

        match app.update(ch('y')) {
            Action::RunCommand(command) => assert_eq!(command, "launch gemma4-12b"),
            other => panic!("expected a plain launch, got {other:?}"),
        }
    }

    /// A preset the machine can hold must not acquire a prompt: friction
    /// that fires on the common case stops being read.
    #[test]
    fn a_preset_that_fits_launches_without_a_prompt() {
        let mut app = app_with_sample();
        app.llama.ram_gib = Some(64);
        app.llama.reserved_ratio = memory::DEFAULT_RESERVED_RATIO;

        match app.update(key(KeyCode::Enter)) {
            Action::RunCommand(command) => assert_eq!(command, "launch gemma4-12b"),
            other => panic!("expected RunCommand, got {other:?}"),
        }
    }

    /// The supervisor can report *that* the system killed the process; only
    /// the App knows what the preset needed and what the budget allowed.
    /// Those two numbers are what make the message actionable.
    #[test]
    fn a_failed_oversized_launch_reports_the_numbers() {
        let mut app = app_with_sample();
        app.llama.ram_gib = Some(10);
        app.llama.reserved_ratio = 0.60;
        app.llama.server.model = Some("gemma4-12b".into());

        app.update(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Error("killed by the system (SIGKILL)".into()),
            LauncherMode::Manual,
            Some("gemma4-12b".into()),
        )));

        let logs = app.logs.iter().cloned().collect::<Vec<_>>().join("\n");
        assert!(logs.contains("GiB budget"), "no sizing context: {logs}");
    }

    /// ...and a model that fits gets no such lecture: a failure that had
    /// nothing to do with memory must not be blamed on memory.
    #[test]
    fn a_failure_that_fits_is_not_blamed_on_memory() {
        let mut app = app_with_sample();
        app.llama.ram_gib = Some(64);
        app.llama.server.model = Some("gemma4-12b".into());

        app.update(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Error("exited with code 1".into()),
            LauncherMode::Manual,
            Some("gemma4-12b".into()),
        )));

        let logs = app.logs.iter().cloned().collect::<Vec<_>>().join("\n");
        assert!(!logs.contains("GiB budget"), "unwarranted blame: {logs}");
    }

    /// The phase is what turns a silent SERVING into a warning the user can
    /// see, so it has to survive the trip from the supervisor to the state.
    #[test]
    fn a_stalled_server_is_marked_degraded() {
        let mut app = app_with_sample();

        app.update(UiEvent::LlamaStatus(
            LlamaSnapshot::new(
                ServerState::Serving,
                LauncherMode::Manual,
                Some("gemma4-12b".into()),
            )
            .with_phase(Phase::Unresponsive(3)),
        ));

        assert!(app.llama.server.is_degraded());
        assert!(app.llama.server.phase.label().is_some());
    }

    #[test]
    fn a_healthy_server_is_not_marked_degraded() {
        let mut app = app_with_sample();

        app.update(UiEvent::LlamaStatus(LlamaSnapshot::new(
            ServerState::Serving,
            LauncherMode::Manual,
            Some("gemma4-12b".into()),
        )));

        assert!(!app.llama.server.is_degraded());
        assert_eq!(app.llama.server.phase.label(), None);
    }

    #[test]
    fn a_port_conflict_asks_before_launching() {
        let mut app = app_with_sample();
        app.update(UiEvent::PortInUse {
            port: 1234,
            name: "gemma4-12b".into(),
            retry: "launch! gemma4-12b".into(),
        });

        assert_eq!(app.mode, Mode::ConfirmLaunch);
        assert_eq!(
            app.llama.confirm,
            Some(Confirm::PortInUse {
                port: 1234,
                retry: "launch! gemma4-12b".into()
            })
        );

        match app.update(ch('y')) {
            Action::RunCommand(command) => assert_eq!(command, "launch! gemma4-12b"),
            other => panic!("expected RunCommand, got {other:?}"),
        }
        assert_eq!(app.mode, Mode::Browse);
    }

    /// The router's version of the same question. The retry command comes
    /// back verbatim — numbers included — rather than being rebuilt from
    /// the prompt's display name, which for the router is not a preset.
    #[test]
    fn a_router_port_conflict_retries_the_router_verbatim() {
        let mut app = app_with_sample();
        app.update(UiEvent::PortInUse {
            port: 1234,
            name: "router".into(),
            retry: "router! --max 3 --idle 120".into(),
        });

        assert_eq!(app.mode, Mode::ConfirmLaunch);

        match app.update(ch('y')) {
            Action::RunCommand(command) => assert_eq!(command, "router! --max 3 --idle 120"),
            other => panic!("expected RunCommand, got {other:?}"),
        }
    }

    #[test]
    fn declining_a_port_conflict_launches_nothing() {
        let mut app = app_with_sample();
        app.update(UiEvent::PortInUse {
            port: 1234,
            name: "gemma4-12b".into(),
            retry: "launch! gemma4-12b".into(),
        });

        assert!(matches!(app.update(ch('n')), Action::None));
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.llama.pending_launch.is_none());
        assert!(!app.running);
    }

    #[test]
    fn models_command_reloads_without_flipping_the_running_flag() {
        let mut app = app_with_sample();
        app.update(ch(':'));
        for c in "models".chars() {
            app.update(ch(c));
        }
        app.update(key(KeyCode::Enter));

        assert!(!app.running, "a local file read must not look async");
        assert_eq!(app.mode, Mode::Browse);
    }

    #[test]
    fn a_command_is_dispatched_to_the_executor() {
        let mut app = app_with_sample();
        app.update(ch(':'));
        for c in "status".chars() {
            app.update(ch(c));
        }

        match app.update(key(KeyCode::Enter)) {
            Action::RunCommand(command) => assert_eq!(command, "status"),
            other => panic!("expected RunCommand, got {other:?}"),
        }
        assert!(app.running);
    }

    #[test]
    fn input_is_ignored_while_a_command_is_in_flight() {
        let mut app = app_with_sample();
        app.running = true;
        assert!(matches!(app.update(key(KeyCode::Enter)), Action::None));
    }

    #[test]
    fn command_finished_clears_the_running_flag() {
        let mut app = app_with_sample();
        app.running = true;
        app.update(UiEvent::CommandFinished {
            command: "status".into(),
            output: "ok".into(),
        });
        assert!(!app.running);
    }

    #[test]
    fn logs_are_capped() {
        let mut app = app_with_sample();
        for i in 0..MAX_LOGS + 50 {
            app.push_log(format!("line {i}"));
        }
        assert_eq!(app.logs.len(), MAX_LOGS);
    }

    #[test]
    fn a_missing_config_is_reported_without_panicking() {
        let app = App::with_config_path(PathBuf::from("/nonexistent/models.ini"));
        assert!(app.llama.config_error.is_some());
        assert!(app.llama.rows().is_empty());
        assert!(app.llama.argv_preview().is_err());
        assert!(app.llama.setting_rows().is_empty());
    }

    #[test]
    fn quit_event_returns_quit_action() {
        let mut app = app_with_sample();
        assert!(matches!(app.update(UiEvent::Quit), Action::Quit));
    }

    #[test]
    fn tick_is_noop() {
        let mut app = app_with_sample();
        assert!(matches!(app.update(UiEvent::Tick), Action::None));
    }
}
