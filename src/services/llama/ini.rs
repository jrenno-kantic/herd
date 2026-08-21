//! Parser + argv builder for `models.ini`, ported from `llama-launch.js`.
//!
//! Differences from the JS version, both intentional:
//! - Values are returned as an argv (`Vec<String>`) rather than a shell
//!   string, so callers spawn `llama-server` directly (`Command::args`)
//!   without any shell in between — no quoting/escaping bugs possible.
//! - Section/flag precedence and "last one wins, keep original position"
//!   semantics are preserved exactly: `[server]` -> `[*]` -> `[model]` ->
//!   CLI overrides.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// One `[section]` of the ini file: an ordered set of `key = value`
/// entries. Re-setting an existing key keeps its original position.
#[derive(Debug, Clone, Default)]
pub struct Section {
    entries: Vec<(String, String)>,
}

impl Section {
    fn set(&mut self, key: &str, value: &str) {
        if let Some(entry) = self.entries.iter_mut().find(|(k, _)| k == key) {
            entry.1 = value.to_string();
        } else {
            self.entries.push((key.to_string(), value.to_string()));
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, Default)]
pub struct LlamaConfig {
    pub path: PathBuf,
    pub server: Section,
    pub defaults: Section,
    /// The optional `[mono-focus]` profile: a named set of flags applied on
    /// top of a preset when the user switches it on, for the case the ini
    /// cannot otherwise express — one client, looping on the same base
    /// prompt, wanting the KV cache kept rather than shared out.
    ///
    /// A **reserved section name**, like `server` and `*`, and that is the
    /// whole of how it is "handled": it is parsed out of `models` so it can
    /// never appear as a launchable preset, be counted in a tier, or be
    /// selected on the Models screen. A section with no `hf-repo` that the
    /// table offered to launch would be a preset that cannot run.
    pub mono_focus: Section,
    pub models: Vec<(String, Section)>,
}

/// The name of that section, spelled once.
pub const MONO_FOCUS: &str = "mono-focus";

impl LlamaConfig {
    pub fn model_names(&self) -> Vec<&str> {
        self.models.iter().map(|(name, _)| name.as_str()).collect()
    }

    pub fn model(&self, name: &str) -> Option<&Section> {
        self.models.iter().find(|(n, _)| n == name).map(|(_, s)| s)
    }

    pub fn host(&self) -> String {
        self.server.get("host").unwrap_or("127.0.0.1").to_string()
    }

    pub fn port(&self) -> u16 {
        self.server
            .get("port")
            .and_then(|value| value.parse().ok())
            .unwrap_or(1234)
    }

    /// Host to use when *connecting to* the server as a client — `0.0.0.0`
    /// (a common bind-all value in `[server]`) isn't a valid target.
    pub fn client_host(&self) -> String {
        match self.host().as_str() {
            "0.0.0.0" | "::" => "127.0.0.1".to_string(),
            other => other.to_string(),
        }
    }
}

/// Resolves which `models.ini` to use, in strict precedence order:
///
/// 1. `--config <path>` on the command line (`cli`),
/// 2. `$HERD_LLAMA_CONFIG`,
/// 3. the RAM tier auto-detected under `~/models/` (`16gb/`, `32gb/`, ...),
/// 4. the legacy flat `~/models/models.ini`.
///
/// Steps 1 and 2 are taken at face value — an explicit choice is never
/// second-guessed, and a path that does not exist surfaces later as a
/// readable "cannot read ..." error rather than being silently replaced.
pub fn resolve_config_path(cli: Option<&Path>) -> PathBuf {
    if let Some(path) = cli {
        return PathBuf::from(expand_tilde(&path.to_string_lossy()));
    }
    if let Some(path) = config_env() {
        return PathBuf::from(expand_tilde(&path));
    }

    let root = PathBuf::from(expand_tilde(MODELS_ROOT));
    if let Some(tier) = pick_tier(&discover_tiers(&root), total_ram_gib()) {
        return tier;
    }

    root.join("models.ini")
}

pub fn default_config_path() -> PathBuf {
    resolve_config_path(None)
}

/// The repo reference a preset launches from, walking the same precedence
/// chain as the argv builder (`[model]` -> `[*]` -> `[server]`).
///
/// Needed at launch so the health poller knows which cache directory to
/// watch for a download in flight.
pub fn effective_repo(config: &LlamaConfig, model: &str) -> Option<String> {
    for key in ["hf-repo", "hf", "model"] {
        let found = config
            .model(model)
            .and_then(|section| section.get(key))
            .or_else(|| config.defaults.get(key))
            .or_else(|| config.server.get(key));

        if let Some(value) = found {
            return Some(value.to_string());
        }
    }
    None
}

/// A `[preset]` stanza for a model that is in the cache but not in the
/// ini, ready to paste into one.
///
/// The minimum that makes a preset launchable, and nothing more: `[*]`
/// already carries the context size, the GPU layers and the rest, and a
/// stanza that restated them would fight the defaults the file is built
/// around. It is offered as text on the clipboard rather than appended to
/// the file, because `models.ini` is hand-maintained and commented and herd
/// does not write to it — see `overrides.rs` for the same rule.
pub fn preset_stanza(reference: &str) -> String {
    let name = preset_name(reference);

    format!("[{name}]\nhf-repo = {reference}\nalias = {name}\n")
}

/// A preset name derived from a repo reference: `unsloth/Qwen3-14B-GGUF`
/// becomes `qwen3-14b`.
///
/// Lower case with the vendor and the `-GGUF` suffix dropped, matching how
/// the shipped tiers name their sections. The quantisation is left out: it
/// is in `hf-repo` on the next line, and a section name is what the user
/// will type at `:launch`.
pub fn preset_name(reference: &str) -> String {
    let repo = super::hub::split_repo(reference).0;
    let tail = repo.rsplit('/').next().unwrap_or(repo).to_ascii_lowercase();
    let tail = tail.trim_end_matches("-gguf").trim_end_matches("_gguf");

    let cleaned: String = tail
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();

    // Runs of punctuation collapse rather than becoming a row of dashes.
    let name = cleaned
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    if name.is_empty() {
        "model".to_string()
    } else {
        name
    }
}

/// The config override from the environment.
///
/// `$OPS_TUI_LLAMA_CONFIG` was honoured alongside this for as long as the
/// rename was recent: the variable was likely to be sitting in a shell
/// profile nobody thinks to update, and ignoring it would have resolved to
/// a *different* tier rather than failing visibly. The migration is done —
/// it is set nowhere on this machine and in no shell profile — so the
/// second name is gone rather than carried indefinitely.
pub fn config_env() -> Option<String> {
    std::env::var("HERD_LLAMA_CONFIG")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// A RAM tier discovered under `~/models/`, for the UI's tier switcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier {
    /// RAM budget advertised by the directory name, in GiB.
    pub gib: u64,
    /// Directory name as shown to the user (`16gb`, `32gb`).
    pub name: String,
    pub config_path: PathBuf,
}

/// Every tier available on this machine, ascending. Empty when `~/models/`
/// holds no `<N>gb/models.ini` at all (the legacy flat layout).
pub fn tiers() -> Vec<Tier> {
    let root = PathBuf::from(expand_tilde(MODELS_ROOT));

    discover_tiers(&root)
        .into_iter()
        .map(|(gib, config_path)| Tier {
            gib,
            name: format!("{gib}gb"),
            config_path,
        })
        .collect()
}

/// Installed RAM in GiB, for display next to the tier list.
pub fn installed_ram_gib() -> Option<u64> {
    total_ram_gib()
}

const MODELS_ROOT: &str = "~/models";

/// Parses a tier directory name (`16gb`, `32GB`) into its RAM budget in GiB.
/// Anything else is not a tier and is ignored during discovery.
fn tier_gib(dir_name: &str) -> Option<u64> {
    dir_name
        .to_ascii_lowercase()
        .strip_suffix("gb")
        .filter(|digits| !digits.is_empty())
        .and_then(|digits| digits.parse().ok())
}

/// Finds every `<N>gb/models.ini` under `root`, sorted by ascending tier.
/// A tier directory without a `models.ini` is not a tier.
fn discover_tiers(root: &Path) -> Vec<(u64, PathBuf)> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut tiers: Vec<(u64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let gib = tier_gib(&entry.file_name().to_string_lossy())?;
            let config = entry.path().join("models.ini");
            config.is_file().then_some((gib, config))
        })
        .collect();

    tiers.sort_by_key(|(gib, _)| *gib);
    tiers
}

/// Picks the richest tier the machine can actually hold. If every tier is
/// bigger than the installed RAM — or the RAM could not be read at all —
/// falls back to the *smallest* tier: attempting the lightest presets is
/// strictly better than refusing to start, and the heavier ones would not
/// have fit anyway.
fn pick_tier(tiers: &[(u64, PathBuf)], ram_gib: Option<u64>) -> Option<PathBuf> {
    let fits = ram_gib.and_then(|ram| tiers.iter().rev().find(|(gib, _)| *gib <= ram));

    fits.or_else(|| tiers.first()).map(|(_, path)| path.clone())
}

/// Installed physical RAM in GiB. Deliberately shells out / reads procfs
/// rather than pulling in a system-info crate for one number.
#[cfg(target_os = "macos")]
fn total_ram_gib() -> Option<u64> {
    let output = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    let bytes: u64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()?;
    Some(bytes / (1024 * 1024 * 1024))
}

#[cfg(target_os = "linux")]
fn total_ram_gib() -> Option<u64> {
    let meminfo = fs::read_to_string("/proc/meminfo").ok()?;
    let kib: u64 = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .trim()
        .strip_suffix("kB")?
        .trim()
        .parse()
        .ok()?;
    Some(kib / (1024 * 1024))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn total_ram_gib() -> Option<u64> {
    None
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    path.to_string()
}

pub fn load(path: &Path) -> Result<LlamaConfig, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    Ok(parse(&text, path))
}

fn parse(text: &str, path: &Path) -> LlamaConfig {
    let mut server = Section::default();
    let mut defaults = Section::default();
    let mut mono_focus = Section::default();
    let mut models: Vec<(String, Section)> = Vec::new();
    let mut current: Option<String> = None;

    for raw_line in text.lines() {
        let line = raw_line.trim();

        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            let name = line[1..line.len() - 1].trim().to_string();
            current = Some(name.clone());
            if !is_reserved(&name) && !models.iter().any(|(n, _)| n == &name) {
                models.push((name, Section::default()));
            }
            continue;
        }

        let Some(section_name) = current.clone() else {
            continue;
        };

        let Some(eq) = line.find('=') else {
            continue;
        };
        let key = line[..eq].trim().to_string();
        let mut value = line[eq + 1..].trim().to_string();

        if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            value = value[1..value.len() - 1].to_string();
        }

        match section_name.as_str() {
            "server" => server.set(&key, &value),
            "*" => defaults.set(&key, &value),
            MONO_FOCUS => mono_focus.set(&key, &value),
            name => {
                if let Some((_, section)) = models.iter_mut().find(|(n, _)| n == name) {
                    section.set(&key, &value);
                }
            }
        }
    }

    LlamaConfig {
        path: path.to_path_buf(),
        server,
        defaults,
        mono_focus,
        models,
    }
}

/// Section names that are not presets. Everything else in the file is one,
/// which is why adding a profile means adding it here — a section the
/// parser does not know becomes a model that cannot launch.
fn is_reserved(name: &str) -> bool {
    matches!(name, "server" | "*" | MONO_FOCUS)
}

/// Mirrors `llama-launch.js`'s two special cases exactly: only the bare
/// `hf` key gets the short `-hf` flag. `hf-repo` (the key actually used
/// throughout the provided `models.ini`) falls through to the default
/// rule and becomes `--hf-repo` — which is llama-server's long-form
/// alias for the same option, so behaviour is unchanged either way, but
/// this keeps the emitted argv identical to what the JS launcher would
/// have produced for the same file.
fn flag_for(key: &str) -> String {
    match key {
        "model" => "-m".to_string(),
        "hf" => "-hf".to_string(),
        _ => format!("--{key}"),
    }
}

/// Ordered flag -> value map mirroring `llama-launch.js`'s `options` Map:
/// re-setting a flag keeps its original position but replaces the value;
/// a `false` ini value removes a previously set flag entirely.
#[derive(Default)]
struct OptionMap {
    order: Vec<String>,
    values: HashMap<String, Option<String>>,
}

impl OptionMap {
    fn set(&mut self, flag: &str, value: Option<String>) {
        if !self.values.contains_key(flag) {
            self.order.push(flag.to_string());
        }
        self.values.insert(flag.to_string(), value);
    }

    fn unset(&mut self, flag: &str) {
        self.order.retain(|f| f != flag);
        self.values.remove(flag);
    }

    fn apply_section(&mut self, section: &Section) {
        for (key, raw) in section.iter() {
            let flag = flag_for(key);
            match raw.to_lowercase().as_str() {
                "true" => self.set(&flag, None),
                "false" => self.unset(&flag),
                _ => self.set(&flag, Some(raw.to_string())),
            }
        }
    }

    fn into_args(self) -> Vec<String> {
        let mut args = Vec::new();
        for flag in self.order {
            args.push(flag.clone());
            if let Some(Some(value)) = self.values.get(&flag) {
                args.push(value.clone());
            }
        }
        args
    }
}

/// What a launch needs that the ini does not carry: the session overrides,
/// and which presets have the `[mono-focus]` profile switched on.
///
/// It exists because the preview and the launch **must build the same
/// argv**, and for a long time they did not: `argv_preview` passed the
/// overrides and `Executor::spawn_launch` did not, so the Models screen
/// showed a `--ctx-size` the spawned process never saw. Both now go
/// through [`LaunchSettings::argv`], which is the only place the pieces
/// are assembled.
#[derive(Debug, Clone, Default)]
pub struct LaunchSettings {
    pub overrides: super::Overrides,
    /// Presets with mono-focus on, by name — the same keying as favourites
    /// and overrides, since a preset is the same model in either tier.
    pub mono_focus: std::collections::BTreeSet<String>,
}

impl LaunchSettings {
    pub fn mono_focus_on(&self, model: &str) -> bool {
        self.mono_focus.contains(model)
    }

    /// The exact argv a launch of `model` would spawn. `cli` is what
    /// followed a bare `--` on a typed `:launch`, and stays last so an
    /// explicit instruction still wins over everything remembered.
    pub fn argv(
        &self,
        config: &LlamaConfig,
        model: &str,
        cli: &[String],
    ) -> Result<Vec<String>, String> {
        let mut extra = self.overrides.to_args(model);
        extra.extend_from_slice(cli);

        build_model_args(config, model, self.mono_focus_on(model), &extra)
    }
}

/// argv for launching a *single* model directly: `llama-server <flags>`,
/// precedence `[server] -> [*] -> [model] -> mono-focus -> CLI`.
///
/// The profile sits **after the preset's own keys and before the override
/// slot**, which is the whole point of it: it is switched on to force a
/// preset into single-client behaviour, so it has to beat what the preset
/// says — while still losing to a Settings-screen override, so any one of
/// its keys can be taken back without editing the file.
pub fn build_model_args(
    config: &LlamaConfig,
    model: &str,
    mono_focus: bool,
    extra: &[String],
) -> Result<Vec<String>, String> {
    let section = config
        .model(model)
        .ok_or_else(|| format!("unknown model '{model}' in {}", config.path.display()))?;

    let mut options = OptionMap::default();
    options.apply_section(&config.server);
    options.apply_section(&config.defaults);
    options.apply_section(section);
    if mono_focus {
        options.apply_section(&config.mono_focus);
    }
    apply_cli_overrides(&mut options, extra);

    Ok(options.into_args())
}

/// argv for the built-in llama-server *router* mode:
/// `llama-server --models-preset <ini> --models-max N --sleep-idle-seconds S`,
/// reusing `[server]` for host/port/jinja/etc.
pub fn build_router_args(
    config: &LlamaConfig,
    models_max: u32,
    sleep_idle_seconds: u32,
    extra: &[String],
) -> Vec<String> {
    let mut options = OptionMap::default();
    options.apply_section(&config.server);

    options.set("--models-preset", Some(config.path.display().to_string()));
    options.set("--models-max", Some(models_max.to_string()));
    options.set("--sleep-idle-seconds", Some(sleep_idle_seconds.to_string()));

    apply_cli_overrides(&mut options, extra);
    options.into_args()
}

fn apply_cli_overrides(options: &mut OptionMap, extra: &[String]) {
    let mut i = 0;
    while i < extra.len() {
        let arg = &extra[i];
        if !arg.starts_with('-') {
            i += 1;
            continue;
        }
        match extra.get(i + 1) {
            Some(value) if !value.starts_with('-') => {
                options.set(arg, Some(value.clone()));
                i += 2;
            }
            _ => {
                options.set(arg, None);
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Environment variables are process-wide, so any test that sets one
    /// has to take this first or it will race the rest of the suite.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn sample() -> LlamaConfig {
        let text = r#"
[server]
host = 0.0.0.0
port = 1234
jinja = true

[*]
ctx-size = 32768
flash-attn = on

[gemma4-12b]
hf-repo = unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL
alias = gemma4-12b
reasoning = off
no-mmproj = true
"#;
        parse(text, Path::new("/tmp/models.ini"))
    }

    #[test]
    fn parses_sections_and_models() {
        let config = sample();
        assert_eq!(config.host(), "0.0.0.0");
        assert_eq!(config.port(), 1234);
        assert_eq!(config.model_names(), vec!["gemma4-12b"]);
    }

    #[test]
    fn tier_gib_accepts_only_ram_shaped_directory_names() {
        assert_eq!(tier_gib("16gb"), Some(16));
        assert_eq!(tier_gib("32GB"), Some(32));
        assert_eq!(tier_gib("128gb"), Some(128));
        assert_eq!(tier_gib("gb"), None);
        assert_eq!(tier_gib("scripts"), None);
        assert_eq!(tier_gib("lmstudio"), None);
        assert_eq!(tier_gib("16"), None);
    }

    fn tiers(gibs: &[u64]) -> Vec<(u64, PathBuf)> {
        gibs.iter()
            .map(|gib| (*gib, PathBuf::from(format!("/models/{gib}gb/models.ini"))))
            .collect()
    }

    #[test]
    fn pick_tier_takes_the_richest_that_fits() {
        let tiers = tiers(&[16, 32, 64]);
        assert_eq!(
            pick_tier(&tiers, Some(32)),
            Some(PathBuf::from("/models/32gb/models.ini"))
        );
        // 48 GiB holds the 32gb presets but not the 64gb ones.
        assert_eq!(
            pick_tier(&tiers, Some(48)),
            Some(PathBuf::from("/models/32gb/models.ini"))
        );
        assert_eq!(
            pick_tier(&tiers, Some(128)),
            Some(PathBuf::from("/models/64gb/models.ini"))
        );
    }

    /// A machine smaller than every tier, or one whose RAM could not be
    /// read, still gets a usable config: the lightest one.
    #[test]
    fn pick_tier_falls_back_to_the_smallest_tier() {
        let tiers = tiers(&[16, 32]);
        assert_eq!(
            pick_tier(&tiers, Some(8)),
            Some(PathBuf::from("/models/16gb/models.ini"))
        );
        assert_eq!(
            pick_tier(&tiers, None),
            Some(PathBuf::from("/models/16gb/models.ini"))
        );
    }

    #[test]
    fn pick_tier_without_any_tier_is_none() {
        assert_eq!(pick_tier(&[], Some(32)), None);
        assert_eq!(pick_tier(&[], None), None);
    }

    /// Discovery must ignore sibling directories that are not tiers
    /// (`scripts/`, `lmstudio/`) and tier-shaped directories that hold no
    /// `models.ini`.
    #[test]
    fn discover_tiers_finds_only_directories_holding_a_models_ini() {
        let root = std::env::temp_dir().join(format!("herd-tiers-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        for dir in ["16gb", "32gb", "scripts", "lmstudio", "empty-gb"] {
            fs::create_dir_all(root.join(dir)).expect("create tier dir");
        }
        for dir in ["16gb", "32gb", "scripts"] {
            fs::write(root.join(dir).join("models.ini"), "[server]\n").expect("write config");
        }

        let found = discover_tiers(&root);
        let _ = fs::remove_dir_all(&root);

        assert_eq!(
            found,
            vec![
                (16, root.join("16gb").join("models.ini")),
                (32, root.join("32gb").join("models.ini")),
            ]
        );
    }

    #[test]
    fn discover_tiers_on_a_missing_root_is_empty() {
        assert!(discover_tiers(Path::new("/nonexistent/herd/models")).is_empty());
    }

    /// One variable now, and an empty one is not an override — that would
    /// resolve to a path of `""` and fail with an unreadable error rather
    /// than falling through to tier detection.
    ///
    /// Serialised, because it mutates process-wide environment.
    #[test]
    fn the_environment_can_name_the_config() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let restore = std::env::var("HERD_LLAMA_CONFIG").ok();

        std::env::remove_var("HERD_LLAMA_CONFIG");
        assert_eq!(config_env(), None);

        std::env::set_var("HERD_LLAMA_CONFIG", "/new/models.ini");
        assert_eq!(config_env().as_deref(), Some("/new/models.ini"));

        std::env::set_var("HERD_LLAMA_CONFIG", "   ");
        assert_eq!(config_env(), None, "blank is not a config path");

        std::env::remove_var("HERD_LLAMA_CONFIG");
        if let Some(value) = restore {
            std::env::set_var("HERD_LLAMA_CONFIG", value);
        }
    }

    #[test]
    fn explicit_cli_path_wins_over_everything() {
        let chosen = resolve_config_path(Some(Path::new("/tmp/explicit/models.ini")));
        assert_eq!(chosen, PathBuf::from("/tmp/explicit/models.ini"));
    }

    #[test]
    fn cli_path_expands_a_leading_tilde() {
        let home = std::env::var("HOME").expect("HOME");
        let chosen = resolve_config_path(Some(Path::new("~/models/16gb/models.ini")));
        assert_eq!(
            chosen,
            PathBuf::from(format!("{home}/models/16gb/models.ini"))
        );
    }

    #[test]
    fn client_host_translates_bind_all() {
        let config = sample();
        assert_eq!(config.client_host(), "127.0.0.1");
    }

    #[test]
    fn build_model_args_applies_precedence_and_booleans() {
        let config = sample();
        let args = build_model_args(&config, "gemma4-12b", false, &[]).unwrap();

        // server + defaults + model flags all present
        assert!(args.contains(&"--host".to_string()));
        assert!(args.contains(&"--ctx-size".to_string()));
        assert!(args.contains(&"--hf-repo".to_string()));
        assert!(args.contains(&"--jinja".to_string()));
        // boolean true emits a bare flag with no following value
        let jinja_idx = args.iter().position(|a| a == "--jinja").unwrap();
        assert_eq!(
            args.get(jinja_idx + 1).map(String::as_str),
            Some("--ctx-size")
        );
        // boolean false (reasoning = off is NOT a recognized bool -> kept as value)
        assert!(args.contains(&"--reasoning".to_string()));
    }

    #[test]
    fn build_model_args_unknown_model_errors() {
        let config = sample();
        assert!(build_model_args(&config, "does-not-exist", false, &[]).is_err());
    }

    #[test]
    fn cli_overrides_take_precedence() {
        let config = sample();
        let extra = vec!["--port".to_string(), "9999".to_string()];
        let args = build_model_args(&config, "gemma4-12b", false, &extra).unwrap();
        let idx = args.iter().position(|a| a == "--port").unwrap();
        assert_eq!(args.get(idx + 1).map(String::as_str), Some("9999"));
    }

    #[test]
    fn build_router_args_includes_preset_flags() {
        let config = sample();
        let args = build_router_args(&config, 3, 120, &[]);
        assert!(args.contains(&"--models-preset".to_string()));
        assert!(args.contains(&"--models-max".to_string()));
        assert!(args.contains(&"3".to_string()));
        assert!(args.contains(&"--sleep-idle-seconds".to_string()));
        assert!(args.contains(&"120".to_string()));
        // router mode also carries the [server] block (host/port/jinja)
        assert!(args.contains(&"--host".to_string()));
    }

    /// Regression test against a trimmed copy of the real `models.ini`
    /// supplied alongside this feature, to make sure the port from
    /// `llama-launch.js` behaves identically on real-world input (in
    /// particular the `hf-repo` key, and the `spec-type`/`spec-draft-n-max`
    /// pair used for MTP draft decoding).
    /// The presets shipped in `data/` are the user's real ones. Parsing
    /// them here — rather than a trimmed inline copy — means a preset that
    /// gains an option shape the parser mishandles fails the suite instead
    /// of only failing at launch time.
    fn shipped_tier(name: &str) -> LlamaConfig {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join(name)
            .join("models.ini");
        load(&path).unwrap_or_else(|error| panic!("load {}: {error}", path.display()))
    }

    #[test]
    fn shipped_16gb_tier_parses_with_every_preset() {
        let config = shipped_tier("16gb");

        assert_eq!(
            config.model_names(),
            vec![
                "qwen3.5-4b",
                "qwen3.5-4b-mtp",
                "qwen3.5-9b",
                "qwen3.5-9b-mtp",
                "qwen3.8-27b",
                "gemma4-e4b",
                "qwen3-4b",
                "gemma3-4b",
                "phi4-mini",
                "nemotron3-nano-4b",
                "gemma4-12b",
                "qwen3-vl-8b-instruct",
                "qwen-3-14b-instruct",
                "bonsai-27b",
            ]
        );
    }

    #[test]
    fn shipped_32gb_tier_parses_with_every_preset() {
        let config = shipped_tier("32gb");

        assert_eq!(
            config.model_names(),
            vec![
                "gemma4-12b",
                "gemma4-26b",
                "gemma4-31b",
                "qwen36-27b",
                "qwen36-35b",
                "huihui-qwen3-6-35b-a3b-abliterated-mtp",
                "qwen38-27b",
                "qwen3-coder",
                "qwen3-vl-8b-instruct",
                "qwen-3-14b-instruct",
            ]
        );
    }

    /// Every shipped preset must build a launchable argv: a model that
    /// cannot produce a command line is a preset the UI would offer and
    /// then fail on.
    #[test]
    fn every_shipped_preset_builds_an_argv() {
        for tier in ["16gb", "32gb"] {
            let config = shipped_tier(tier);

            for name in config.model_names() {
                let argv = build_model_args(&config, name, false, &[])
                    .unwrap_or_else(|error| panic!("{tier}/{name}: {error}"));

                assert!(
                    argv.windows(2).any(|w| w == ["--alias", name])
                        || argv.iter().any(|token| token == "--alias"),
                    "{tier}/{name}: no --alias in argv"
                );
                assert!(
                    argv.iter()
                        .any(|token| token == "--hf-repo" || token == "-hf" || token == "-m"),
                    "{tier}/{name}: no model source in argv"
                );
            }
        }
    }

    /// Both shipped tiers bind the same port, which is exactly why the
    /// launcher has a port-conflict prompt. If this ever stops being true
    /// the prompt is still correct, but the docs explaining it are not.
    #[test]
    fn shipped_tiers_share_a_port() {
        assert_eq!(shipped_tier("16gb").port(), shipped_tier("32gb").port());
    }

    const WITH_PROFILE: &str = r#"
[server]
host = 0.0.0.0
port = 1234

[*]
ctx-size = 32768
parallel = 4

[mono-focus]
cache-type-k = q8_0
cache-type-v = q8_0
parallel = 1
cache-reuse = 256
keep = -1

[gemma4-12b]
hf-repo = unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL
parallel = 2
"#;

    /// The reserved section is not a preset. Left in `models` it would be
    /// offered on the Models screen, counted in the tier, and launchable —
    /// a preset with no `hf-repo`, which cannot run.
    #[test]
    fn the_profile_section_is_never_a_preset() {
        let config = parse(WITH_PROFILE, Path::new("test.ini"));

        assert_eq!(config.model_names(), vec!["gemma4-12b"]);
        assert!(config.model(MONO_FOCUS).is_none());
        assert_eq!(config.mono_focus.get("cache-reuse"), Some("256"));
    }

    /// Off by default, and off means absent from the argv — not a flag
    /// with a different value.
    #[test]
    fn the_profile_changes_nothing_until_it_is_switched_on() {
        let config = parse(WITH_PROFILE, Path::new("test.ini"));
        let args = build_model_args(&config, "gemma4-12b", false, &[]).expect("argv");

        assert!(!args.iter().any(|a| a == "--cache-reuse"), "{args:?}");
        assert!(!args.iter().any(|a| a == "--cache-type-k"), "{args:?}");
        // The preset's own value stands.
        assert_eq!(value_after(&args, "--parallel"), Some("2"));
    }

    /// Switched on it **beats the preset**, which is the point: it is
    /// turned on to force single-client behaviour onto a model whose own
    /// section says otherwise.
    #[test]
    fn the_profile_overrides_the_presets_own_keys() {
        let config = parse(WITH_PROFILE, Path::new("test.ini"));
        let args = build_model_args(&config, "gemma4-12b", true, &[]).expect("argv");

        assert_eq!(value_after(&args, "--parallel"), Some("1"));
        assert_eq!(value_after(&args, "--cache-type-k"), Some("q8_0"));
        assert_eq!(value_after(&args, "--keep"), Some("-1"));
        // ...and leaves everything it does not mention alone.
        assert_eq!(value_after(&args, "--ctx-size"), Some("32768"));
    }

    /// ...while still losing to an override, so any one of its keys can be
    /// taken back from the Settings screen without editing the file.
    #[test]
    fn an_override_still_beats_the_profile() {
        let config = parse(WITH_PROFILE, Path::new("test.ini"));
        let mut settings = LaunchSettings {
            mono_focus: ["gemma4-12b".to_string()].into(),
            ..LaunchSettings::default()
        };
        settings.overrides.set(
            super::super::overrides::Scope::Model,
            "gemma4-12b",
            "parallel",
            "3",
        );

        let args = settings.argv(&config, "gemma4-12b", &[]).expect("argv");
        assert_eq!(value_after(&args, "--parallel"), Some("3"));
        // The rest of the profile is untouched by that one override.
        assert_eq!(value_after(&args, "--cache-reuse"), Some("256"));
    }

    /// The bug this plumbing exists to close: `argv_preview` applied the
    /// session overrides and `Executor::spawn_launch` did not, so the
    /// Models screen drew a `--ctx-size` the spawned process never saw.
    /// Both call `LaunchSettings::argv` now, and this pins that the
    /// overrides survive it.
    #[test]
    fn the_launch_argv_carries_the_session_overrides() {
        let config = parse(WITH_PROFILE, Path::new("test.ini"));
        let mut settings = LaunchSettings::default();
        settings.overrides.set(
            super::super::overrides::Scope::Model,
            "gemma4-12b",
            "ctx-size",
            "65536",
        );

        let args = settings.argv(&config, "gemma4-12b", &[]).expect("argv");
        assert_eq!(value_after(&args, "--ctx-size"), Some("65536"));
    }

    /// An explicit `:launch model -- --flag` still wins over everything
    /// remembered: it is an instruction, not a preference.
    #[test]
    fn a_typed_flag_beats_the_overrides_and_the_profile() {
        let config = parse(WITH_PROFILE, Path::new("test.ini"));
        let settings = LaunchSettings {
            mono_focus: ["gemma4-12b".to_string()].into(),
            ..LaunchSettings::default()
        };

        let cli = vec!["--parallel".to_string(), "8".to_string()];
        let args = settings.argv(&config, "gemma4-12b", &cli).expect("argv");

        assert_eq!(value_after(&args, "--parallel"), Some("8"));
    }

    /// The value argv carries after a flag, or `None` when the flag is
    /// absent — the assertions above are about precedence, not position.
    fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        let at = args.iter().position(|a| a == flag)?;
        args.get(at + 1).map(String::as_str)
    }

    /// The shipped tiers carry the profile now, and it must not have
    /// turned into a fourteenth preset — which is exactly what the two
    /// `shipped_*_tier_parses_with_every_preset` tests would catch, and
    /// this states outright.
    #[test]
    fn the_shipped_tiers_carry_the_profile_without_gaining_a_preset() {
        for tier in ["16gb", "32gb"] {
            let config = shipped_tier(tier);

            assert_eq!(config.mono_focus.get("parallel"), Some("1"), "{tier}");
            assert_eq!(config.mono_focus.get("cache-reuse"), Some("256"), "{tier}");
            // A boolean flag, not a count: `slots = 1` would emit
            // `--slots 1` and leave a stray argument behind.
            assert_eq!(config.mono_focus.get("slots"), Some("true"), "{tier}");
            assert!(
                !config.model_names().contains(&MONO_FOCUS),
                "{tier} offers the profile as a preset"
            );
        }
    }

    #[test]
    fn parses_real_world_models_ini() {
        let text = r#"
[server]
host = 0.0.0.0
port = 1234
jinja = true

[*]
ctx-size = 32768
gpu-layers = 99
flash-attn = on

[gemma4-12b]
hf-repo = unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL
alias = gemma4-12b
reasoning = off
spec-type = draft-mtp
spec-draft-n-max = 4
no-mmproj = true

[qwen3-coder]
hf-repo = unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:UD-Q4_K_XL
alias = qwen3-coder
reasoning = off
"#;
        let config = parse(text, Path::new("/tmp/models.ini"));

        assert_eq!(config.model_names(), vec!["gemma4-12b", "qwen3-coder"]);
        assert_eq!(config.port(), 1234);

        let args = build_model_args(&config, "gemma4-12b", false, &[]).unwrap();
        assert_eq!(
            args,
            vec![
                "--host",
                "0.0.0.0",
                "--port",
                "1234",
                "--jinja",
                "--ctx-size",
                "32768",
                "--gpu-layers",
                "99",
                "--flash-attn",
                "on",
                "--hf-repo",
                "unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL",
                "--alias",
                "gemma4-12b",
                "--reasoning",
                "off",
                "--spec-type",
                "draft-mtp",
                "--spec-draft-n-max",
                "4",
                "--no-mmproj",
            ]
        );

        let router = build_router_args(&config, 2, 300, &[]);
        assert_eq!(
            router,
            vec![
                "--host",
                "0.0.0.0",
                "--port",
                "1234",
                "--jinja",
                "--models-preset",
                "/tmp/models.ini",
                "--models-max",
                "2",
                "--sleep-idle-seconds",
                "300",
            ]
        );
    }
}
