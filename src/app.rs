use crate::event::UiEvent;
use crate::services::llama::{
    self, api::ChatOutcome, hub::Availability, ini::LlamaConfig, memory, overrides::Scope, Budget,
    Fit, LauncherMode, Overrides, Phase, ServerState, Tier,
};
use chrono::{DateTime, Local};
use crossterm::event::{KeyCode, KeyEvent};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;

const MAX_LOGS: usize = 500;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Screen {
    Models,
    Server,
    Test,
    Stats,
    Settings,
    Logs,
}

impl Screen {
    pub const ALL: [Screen; 6] = [
        Screen::Models,
        Screen::Server,
        Screen::Test,
        Screen::Stats,
        Screen::Settings,
        Screen::Logs,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Screen::Models => "Models",
            Screen::Server => "Server",
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
    /// Quitting would abandon work in flight. Waiting for an answer.
    ConfirmQuit,
    /// The `?` overlay. A mode rather than a screen so it can be summoned
    /// from anywhere and dismissed back to where the user was.
    Help,
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
}

impl SessionStats {
    fn begin(&mut self) {
        *self = Self {
            started_at: Some(Local::now()),
            ..Self::default()
        };
    }

    fn record(&mut self, outcome: &ChatOutcome) {
        self.probes += 1;
        self.prompt_tokens += outcome.prompt_tokens.unwrap_or(0);
        self.completion_tokens += outcome.completion_tokens.unwrap_or(0);
        self.total_latency += outcome.latency;
        self.last_rate = outcome.tokens_per_second;

        if let Some(rate) = outcome.tokens_per_second {
            self.best_rate = Some(self.best_rate.map_or(rate, |best| best.max(rate)));
        }
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
    /// Something herd did not start already holds the port.
    PortInUse(u16),
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
    pub settings_cursor: usize,
    pub edit_buffer: String,
    pub server: ServerRuntime,
    /// Model awaiting confirmation, and what the launcher is asking about.
    pub pending_launch: Option<String>,
    pub confirm: Option<Confirm>,
    /// What `llama-server --cache-list` last reported. `None` means we
    /// have not managed to ask, which is why `availability` answers
    /// `Unknown` rather than "missing" — the same restraint as `Fit`.
    pub cached: Option<Vec<String>>,
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
}

impl LauncherState {
    fn new(config_path: PathBuf, last_launched: Option<String>) -> Self {
        let mut state = Self {
            config_path,
            config: None,
            config_error: None,
            tiers: llama::tiers(),
            ram_gib: llama::ini::installed_ram_gib(),
            cursor: 0,
            filter: String::new(),
            overrides: Overrides::default(),
            settings_cursor: 0,
            edit_buffer: String::new(),
            server: ServerRuntime::default(),
            pending_launch: None,
            confirm: None,
            cached: None,
            download: None,
            last_launched,
            prompt: llama::api::DEFAULT_PROMPT.to_string(),
            chat: None,
            chat_pending: false,
            chat_started: None,
            stats: SessionStats::default(),
            reserved_ratio: memory::DEFAULT_RESERVED_RATIO,
            picker_cursor: 0,
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

        llama::ini::build_model_args(config, &model, &self.overrides.to_args(&model))
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

        rows
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

    /// Estimated resident size of a preset, or `None` when its name
    /// carries no parseable parameter count.
    pub fn estimate_gib(&self, name: &str) -> Option<f64> {
        let config = self.config.as_ref()?;
        let repo = effective(config, name, &["hf-repo", "hf", "model"])?;
        memory::estimate_gib(&repo)
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

/// Rows a page key jumps. A constant rather than the viewport height
/// because `App::update` is pure and never learns how tall the terminal
/// is — and a fixed, predictable jump is easier to aim than one that
/// changes with the window.
/// Where a movement key takes a cursor over `len` rows, or `None` if the
/// key is not a movement key and the caller should go on matching it.
///
/// Shared by the Models table, the Settings rows and the config picker, so
/// the three cannot drift apart and every list answers to the same keys.
/// The picker used to hand-roll its own `j`/`k` pair and so quietly lacked
/// page, home and end.
fn moved(cursor: usize, len: usize, key: KeyCode, page: usize) -> Option<usize> {
    let last = len.checked_sub(1)?;

    let next = match key {
        KeyCode::Down | KeyCode::Char('j') => cursor.saturating_add(1).min(last),
        KeyCode::Up | KeyCode::Char('k') => cursor.saturating_sub(1),
        KeyCode::PageDown => cursor.saturating_add(page).min(last),
        KeyCode::PageUp => cursor.saturating_sub(page),
        KeyCode::Home | KeyCode::Char('g') => 0,
        KeyCode::End | KeyCode::Char('G') => last,
        _ => return None,
    };

    Some(next)
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

/// Terminal height assumed before the first `Resize`. The classic 24x80,
/// so a page is sane even if the size never arrives.
const DEFAULT_ROWS: u16 = 24;

/// Rows a screen spends on things that are not list entries: the command
/// bar and status line, the block borders, and whatever the screen puts
/// above or below its list. Subtracted from the terminal height to get the
/// number of rows a page key should move by.
///
/// Paging by more than fits on screen skips content silently, which is the
/// one outcome worth designing against — so where a number is uncertain it
/// errs on the high side and pages slightly short instead.
fn chrome(screen: Screen) -> u16 {
    match screen {
        // command bar (3) + status (1) + argv preview (8) + borders (2)
        // + column header (1) + two footer lines (2)
        Screen::Models => 17,
        // command bar (3) + status (1) + borders (2) + footer (1), plus 6
        // for the section headers. The Settings cursor counts *entries*,
        // but a header is drawn as a blank line and a title — two rows
        // that no entry index accounts for. There are at most three
        // ([server], [*], the preset), so reserving their six rows keeps a
        // page inside the screen whatever the sections contain.
        Screen::Settings => 7 + 6,
        // command bar (3) + status (1) + borders (2) + position footer (1)
        Screen::Logs => 7,
        // No list to page through; the value is unused but must be sane.
        Screen::Server | Screen::Test | Screen::Stats => 12,
    }
}

impl App {
    /// Resolves the `models.ini` path itself (env / RAM tier / legacy).
    /// `main.rs` uses [`App::with_config_path`] instead so the same path is
    /// shared with the `Executor` and a `--config` flag can override it.
    pub fn new() -> Self {
        Self::with_config_path(llama::default_config_path())
    }

    pub fn with_config_path(config_path: PathBuf) -> Self {
        Self::restored(config_path, None)
    }

    /// Builds the app with a remembered preset preselected.
    pub fn restored(config_path: PathBuf, last_launched: Option<String>) -> Self {
        let mut logs = VecDeque::with_capacity(MAX_LOGS);
        logs.push_back("HERD started".into());

        Self {
            command_input: String::new(),
            screen: Screen::Models,
            mode: Mode::Browse,
            logs,
            log_scroll: 0,
            running: false,
            rows: DEFAULT_ROWS,
            llama: LauncherState::new(config_path, last_launched),
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
        const OVERLAP: u16 = 1;
        const MIN_PAGE: u16 = 3;

        self.rows
            .saturating_sub(chrome(self.screen))
            .saturating_sub(OVERLAP)
            .max(MIN_PAGE) as usize
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
            UiEvent::PortInUse { port, model } => {
                self.ask(model, Confirm::PortInUse(port));
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
            UiEvent::Resize { height, .. } => {
                self.rows = height;
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
            Mode::Help => self.handle_help_key(key),
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
            KeyCode::Char(c @ '1'..='6') => {
                let index = c as usize - '1' as usize;
                self.screen = Screen::ALL[index];
                Action::None
            }
            _ => match self.screen {
                Screen::Models => self.handle_models_key(key),
                Screen::Server => self.handle_server_key(key),
                Screen::Test => self.handle_test_key(key),
                Screen::Stats => self.handle_stats_key(key),
                Screen::Settings => self.handle_settings_key(key),
                Screen::Logs => self.handle_logs_key(key),
            },
        }
    }

    /// Any key dismisses the overlay: it is a reference card, and hunting
    /// for the one key that closes it would be its own small joke.
    fn handle_help_key(&mut self, _key: KeyEvent) -> Action {
        self.mode = Mode::Browse;
        Action::None
    }

    fn handle_models_key(&mut self, key: KeyEvent) -> Action {
        if let Some(cursor) = moved(
            self.llama.cursor,
            self.llama.rows().len(),
            key.code,
            self.page(),
        ) {
            self.llama.cursor = cursor;
            // The Settings screen shows the selected preset's keys, so its
            // cursor has to stay in range when the selection moves.
            self.llama.clamp_settings_cursor();
            return Action::None;
        }

        match key.code {
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
            KeyCode::Char('s') => self.stop_server(),
            _ => Action::None,
        }
    }

    fn handle_server_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('s') => self.stop_server(),
            KeyCode::Char('p') => match self.llama.server.model.clone() {
                Some(model) => self.run(format!("ping {model}")),
                None => Action::None,
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

        if let Some(cursor) = moved(
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
        if let Some(cursor) = moved(
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

    /// `q`: quits, or asks first when something would be lost.
    fn quit(&mut self) -> Action {
        if self.in_flight().is_empty() {
            return Action::Quit;
        }

        self.mode = Mode::ConfirmQuit;
        Action::None
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
            // `launch!` skips the port check, which is exactly and only
            // what the user answered here.
            (true, Some(model), Some(Confirm::PortInUse(_))) => {
                self.run(format!("launch! {model}"))
            }
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
        App::with_config_path(path)
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
        app.update(UiEvent::Resize { height: 22 });
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

        app.update(UiEvent::Resize { height: 24 });
        let short = app.page();

        app.update(UiEvent::Resize { height: 60 });
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
        app.update(UiEvent::Resize { height: 1 });

        assert!(app.page() >= 3, "page collapsed to {}", app.page());
    }

    /// Every screen sits under the same command bar, status line and
    /// block borders, so none of them can honestly claim less chrome than
    /// that — a screen that did would page past its own last visible row.
    #[test]
    fn no_screen_claims_less_chrome_than_the_shared_layout() {
        const SHARED: u16 = 3 + 1 + 2; // command bar, status line, borders

        for screen in Screen::ALL {
            assert!(
                chrome(screen) >= SHARED,
                "{screen:?} claims {} rows of chrome, less than the shared {SHARED}",
                chrome(screen)
            );
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
    fn q_quits_from_browse_mode() {
        let mut app = app_with_sample();
        assert!(matches!(app.update(ch('q')), Action::Quit));
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
        for expected in [
            Screen::Server,
            Screen::Test,
            Screen::Stats,
            Screen::Settings,
            Screen::Logs,
            Screen::Models,
        ] {
            app.update(key(KeyCode::Tab));
            assert_eq!(app.screen, expected);
        }
    }

    #[test]
    fn digits_jump_straight_to_a_screen() {
        let mut app = app_with_sample();
        app.update(ch('5'));
        assert_eq!(app.screen, Screen::Settings);
        app.update(ch('3'));
        assert_eq!(app.screen, Screen::Test);
        app.update(ch('1'));
        assert_eq!(app.screen, Screen::Models);
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

        assert_eq!(headers, vec!["[server]", "[*]  defaults", "[gemma4-12b]"]);
        assert!(app.llama.setting_entry_indices().len() >= 7);
    }

    #[test]
    fn editing_a_setting_records_a_session_override() {
        let mut app = app_with_sample();
        app.update(ch('5')); // Settings
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
        app.update(ch('5')); // Settings

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
        app.update(ch('5'));
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

        app.update(ch('5'));
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
        app.update(ch('5'));
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

        app.update(ch('5'));
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
        app = App::restored(path, Some("qwen3-coder".into()));

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
        app.update(ch('3')); // Test

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
        app.update(ch('3'));

        assert!(matches!(app.update(key(KeyCode::Enter)), Action::None));
        assert!(!app.llama.chat_pending);
    }

    #[test]
    fn a_second_probe_is_refused_while_one_is_in_flight() {
        let mut app = app_with_sample();
        serving(&mut app, "gemma4-12b");
        app.update(ch('3'));
        app.update(key(KeyCode::Enter));

        assert!(matches!(app.update(key(KeyCode::Enter)), Action::None));
    }

    #[test]
    fn the_prompt_can_be_edited_and_is_used_for_the_next_probe() {
        let mut app = app_with_sample();
        serving(&mut app, "gemma4-12b");
        app.update(ch('3'));
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
        app.update(ch('3'));
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
        app.update(ch('3'));
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
        app.update(ch('3'));
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
        app.update(ch('3'));
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
        assert_eq!(tiers[1].presets, 8, "32gb preset count");
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
                assert_eq!(app.llama.model_names().len(), 8);
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
        app.update(ch('4')); // Stats

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
        app.update(ch('4'));

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
        app.update(ch('4'));
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
        app.update(ch('4'));
        app.update(ch('-'));
        app.update(ch('r'));

        assert_eq!(app.llama.reserved_ratio, memory::DEFAULT_RESERVED_RATIO);
        assert!(!app.llama.budget().is_risky());
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
            "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL".to_string(),
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
            "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL".to_string(),
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

    /// An idle app quits on `q` with no ceremony — a prompt on the normal
    /// case would train the user to dismiss it unread.
    #[test]
    fn quitting_with_nothing_running_does_not_ask() {
        let mut app = app_with_sample();

        assert!(app.in_flight().is_empty());
        assert!(matches!(app.update(ch('q')), Action::Quit));
    }

    /// A supervised server is deliberately *not* work in flight: stopping
    /// it on exit is the documented behaviour, every time.
    #[test]
    fn a_running_server_alone_is_not_a_reason_to_ask() {
        let mut app = app_with_sample();
        app.llama.server.state = ServerState::Serving;

        assert!(app.in_flight().is_empty());
        assert!(matches!(app.update(ch('q')), Action::Quit));
    }

    #[test]
    fn quitting_mid_download_asks_first() {
        let mut app = app_with_sample();
        app.update(UiEvent::DownloadProgress {
            model: "gemma4-12b".into(),
            done: 1_000,
            total: 4_000,
        });

        assert!(matches!(app.update(ch('q')), Action::None));
        assert_eq!(app.mode, Mode::ConfirmQuit);
        assert!(app.in_flight()[0].contains("gemma4-12b"));

        match app.update(ch('y')) {
            Action::Quit => {}
            other => panic!("y must quit, got {other:?}"),
        }
    }

    #[test]
    fn declining_the_quit_prompt_stays_put() {
        let mut app = app_with_sample();
        app.llama.chat_pending = true;

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
            "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL".to_string(),
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
    fn d_on_an_already_local_preset_does_nothing() {
        let mut app = app_with_sample();
        app.update(UiEvent::CacheList(vec![
            "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL".to_string(),
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
            model: "gemma4-12b".into(),
        });

        assert_eq!(app.mode, Mode::ConfirmLaunch);
        assert_eq!(app.llama.confirm, Some(Confirm::PortInUse(1234)));

        match app.update(ch('y')) {
            Action::RunCommand(command) => assert_eq!(command, "launch! gemma4-12b"),
            other => panic!("expected RunCommand, got {other:?}"),
        }
        assert_eq!(app.mode, Mode::Browse);
    }

    #[test]
    fn declining_a_port_conflict_launches_nothing() {
        let mut app = app_with_sample();
        app.update(UiEvent::PortInUse {
            port: 1234,
            model: "gemma4-12b".into(),
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
