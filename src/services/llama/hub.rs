//! Is the model a preset names actually on this machine, and if not, what
//! would it take to fetch it?
//!
//! Two questions, two different authorities, deliberately:
//!
//! - **"Is it here?"** is answered by `llama-server --cache-list`, not by
//!   looking at the filesystem. llama.cpp is the thing that will have to
//!   load the file, so its opinion is the only one that matters — and it
//!   is better informed than ours. On this machine it correctly refuses to
//!   list a repo whose blobs directory holds a half-finished
//!   `.downloadInProgress` file, which no directory listing of ours would
//!   have caught.
//! - **"What would it cost?"** is answered by the HuggingFace tree API,
//!   which gives a size and a blob id per file. The sizes are needed
//!   before anything starts, since they are what the confirmation prompt
//!   is asking the user to agree to.
//!
//! Fetching is delegated to the `hf` CLI. Writing the hub cache layout
//! correctly — blobs, snapshot symlinks, refs — is the part that must not
//! be got wrong, and `hf` owns that format. What `hf` will not give us is
//! progress: through a pipe it prints a file *count* ("Fetching 3 files:
//! 33%") and no byte counter at all, so a 6.7 GB weights file would sit at
//! 0% for its whole download. Progress is therefore measured rather than
//! parsed — see [`downloaded_bytes`].

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// The CLI that does the fetching.
pub const DOWNLOADER: &str = "hf";

/// Whether the weights a preset names are on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// llama.cpp lists it: launching will not download anything.
    Local,
    /// llama.cpp does not list it: launching would fetch it first.
    Missing,
    /// We have not been able to ask, so nothing is claimed. Same principle
    /// as `Fit::Unknown`: a wrong "you need to download this" is worse
    /// than saying nothing.
    Unknown,
}

impl Availability {
    pub fn label(self) -> Option<&'static str> {
        match self {
            Availability::Local => None,
            Availability::Missing => Some("not local"),
            Availability::Unknown => None,
        }
    }
}

/// Splits `unsloth/Qwen3-14B-GGUF:UD-Q4_K_XL` into its repo and quant tag.
pub fn split_repo(reference: &str) -> (&str, Option<&str>) {
    match reference.split_once(':') {
        Some((repo, tag)) if !tag.is_empty() => (repo.trim(), Some(tag.trim())),
        _ => (reference.trim(), None),
    }
}

/// One model llama.cpp reports in the local hub cache.
///
/// The size is an `Option` rather than a `0` default for the usual reason:
/// a cache directory that cannot be read and one that is empty are
/// different answers, and only one of them is worth printing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedModel {
    /// `repo:quant`, exactly as `--cache-list` spells it — which is not
    /// always how the ini spells it (see [`availability`]).
    pub reference: String,
    /// What this model's **repo** occupies in the hub cache. Per repo
    /// rather than per quantisation because that is the unit the cache
    /// stores: one `blobs/` directory per repo, with no per-quant
    /// accounting in it. A repo holding two quantisations therefore
    /// reports the same total on both, which the UI marks as shared rather
    /// than splitting into a number nobody measured.
    ///
    /// It is disk usage, not model size, and the two are not close: this
    /// machine's `models--unsloth--gemma-4-12B-it-qat-GGUF` holds *two*
    /// 6.7 GiB revisions of the same weights, because a repo that moves on
    /// leaves the old blobs behind. That gap is the whole reason the
    /// number is worth showing.
    pub bytes: Option<u64>,
    /// What llama.cpp would actually load for this entry: the weight files
    /// of the **current revision** matching this quantisation, resolved
    /// through the snapshot symlinks.
    ///
    /// Separate from `bytes` because they answer different questions —
    /// "what would this cost me in memory" against "what would deleting it
    /// give me back" — and on a repo with a stale revision they differ by
    /// a factor of two.
    pub weights: Option<u64>,
}

impl CachedModel {
    pub fn repo(&self) -> &str {
        split_repo(&self.reference).0
    }

    pub fn tag(&self) -> Option<&str> {
        split_repo(&self.reference).1
    }

    /// Do two entries live in the same repo directory, and so share a size?
    pub fn same_repo(&self, other: &CachedModel) -> bool {
        self.repo().eq_ignore_ascii_case(other.repo())
    }
}

/// An entry whose size has not been measured. The form the parser produces
/// and the one the tests use.
impl From<&str> for CachedModel {
    fn from(reference: &str) -> Self {
        Self::from(reference.to_string())
    }
}

impl From<String> for CachedModel {
    fn from(reference: String) -> Self {
        Self {
            reference,
            bytes: None,
            weights: None,
        }
    }
}

/// Reads `llama-server --cache-list` output into `repo:quant` entries.
///
/// The first line is a count ("number of models in cache: 10") and the
/// rest are numbered. Anything unparseable is skipped rather than guessed
/// at — a mangled line must not become a phantom cached model.
///
/// Sizes are left unmeasured here: this is a pure parse, and reading the
/// cache directory is [`measure`]'s job.
pub fn parse_cache_list(output: &str) -> Vec<CachedModel> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (index, rest) = line.split_once('.')?;
            index.trim().parse::<u32>().ok()?;
            let entry = rest.trim();
            (!entry.is_empty() && entry.contains('/')).then(|| CachedModel::from(entry))
        })
        .collect()
}

/// Fills in what each cached repo occupies on disk.
///
/// Separate from the parse so the parse stays testable without a cache,
/// and so a machine with no readable hub directory simply keeps `None`
/// everywhere instead of reporting zeroes.
pub fn measure(entries: &mut [CachedModel]) {
    let Some(hub) = hub_dir() else {
        return;
    };

    for entry in entries {
        let (repo, tag) = split_repo(&entry.reference);
        let dir = repo_dir(&hub, repo);

        entry.bytes = repo_bytes(&dir);
        entry.weights = snapshot_weights_bytes(&dir, tag);
    }
}

/// Bytes held in a repo's blobs directory, or `None` when it cannot be
/// read at all.
///
/// The blobs are the weights; the snapshot directory beside them is
/// symlinks into these, so counting both would double every model.
pub fn repo_bytes(repo_dir: &Path) -> Option<u64> {
    let entries = std::fs::read_dir(repo_dir.join("blobs")).ok()?;

    Some(
        entries
            .flatten()
            .filter_map(|entry| entry.metadata().ok())
            .filter(|meta| meta.is_file())
            .map(|meta| meta.len())
            .sum(),
    )
}

/// The weights of one quantisation, as they sit in the cache **now**.
///
/// Read through `refs/main` and the snapshot directory rather than by
/// summing blobs, because a repo keeps every revision it has ever fetched:
/// on this machine `gemma-4-12B-it-qat-GGUF` holds two 6.7 GiB copies of
/// the same file, and adding them up would announce a 12B model at 13.4
/// GiB. The snapshot names exactly the file llama.cpp would open, and
/// `metadata` follows the symlink to the blob behind it.
///
/// `None` when the revision, the directory or a matching file cannot be
/// found — the same restraint as everywhere else here: a size that is not
/// known is not a size of zero, and the caller falls back to the estimate.
pub fn snapshot_weights_bytes(repo_dir: &Path, tag: Option<&str>) -> Option<u64> {
    let needle = tag_needle(tag)?;
    let dir = current_snapshot(repo_dir)?;

    let candidates: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().to_string(),
                entry.path(),
            )
        })
        .filter(|(name, _)| is_weight_file(name) && matches_tag(stem_of(name), &needle))
        .map(|(name, path)| (stem_of(&name).to_string(), path))
        .collect();

    let chosen = keep_shortest_base(candidates, &needle);
    if chosen.is_empty() {
        return None;
    }

    // A split model is several files and one model; anything that failed to
    // resolve is skipped rather than counted as nothing, so a half-linked
    // snapshot under-reports instead of inventing a size.
    Some(
        chosen
            .iter()
            .filter_map(|path| std::fs::metadata(path).ok())
            .map(|meta| meta.len())
            .sum(),
    )
}

/// The snapshot directory `refs/main` points at.
///
/// Falls back to the only snapshot there is when the ref cannot be read: a
/// repo with exactly one revision is unambiguous, and refusing to answer
/// there would lose the measurement for no gain. With several and no ref,
/// nothing is claimed — guessing which revision is current is guessing the
/// answer.
fn current_snapshot(repo_dir: &Path) -> Option<PathBuf> {
    let snapshots = repo_dir.join("snapshots");

    if let Ok(revision) = std::fs::read_to_string(repo_dir.join("refs").join("main")) {
        let revision = revision.trim();
        if !revision.is_empty() && snapshots.join(revision).is_dir() {
            return Some(snapshots.join(revision));
        }
    }

    let mut dirs = std::fs::read_dir(&snapshots)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir());

    match (dirs.next(), dirs.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// What a cached entry of `reference` was measured at, if one is cached
/// under the *same* quantisation.
///
/// The tag has to match, unlike [`availability`], which deliberately
/// accepts a repo cached under any tag. The two answer different
/// questions: "will launching download anything" is about the repo, but
/// "how much memory will this take" is about the exact build — reporting a
/// cached Q8's size against a preset asking for Q4 would be a confident,
/// specific and wrong number, which is worse than falling back to the
/// estimate the module already documents as one.
pub fn measured_weights(reference: &str, cached: &[CachedModel]) -> Option<u64> {
    let (repo, tag) = split_repo(reference);
    let wanted = tag.map(normalised_tag)?;

    cached
        .iter()
        .filter(|entry| entry.repo().eq_ignore_ascii_case(repo))
        .find(|entry| entry.tag().map(normalised_tag).as_deref() == Some(wanted.as_str()))
        .and_then(|entry| entry.weights)
}

/// A quant tag as both spellings agree on it.
///
/// The ini says `UD-Q4_K_XL` where `--cache-list` reports `Q4_K_XL`,
/// llama.cpp having dropped Unsloth's dynamic-quantisation prefix. They are
/// the same build, and only compare equal once that prefix is off both.
fn normalised_tag(tag: &str) -> String {
    let tag = tag.trim().to_ascii_lowercase();
    tag.strip_prefix("ud-").unwrap_or(&tag).to_string()
}

/// Is `reference` (an ini `hf-repo` value) among the cached entries?
///
/// The two spellings do not match literally: the ini says
/// `unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL` where the cache reports
/// `...:Q4_K_XL`, llama.cpp having dropped Unsloth's `UD-` prefix. So the
/// tag is compared with that prefix stripped from both sides.
///
/// A repo that is cached under *some other* tag counts as available. The
/// alternative is telling the user to download something they may well
/// already have because a naming convention moved — and being wrong in
/// that direction is the more expensive mistake, since the launch itself
/// will fetch anything genuinely absent.
pub fn availability(reference: &str, cached: &[CachedModel]) -> Availability {
    let (repo, tag) = split_repo(reference);

    let same_repo = |entry: &CachedModel| entry.repo().eq_ignore_ascii_case(repo);

    match (cached.iter().any(same_repo), tag) {
        (false, _) => Availability::Missing,
        (true, _) => Availability::Local,
    }
}

/// Asks llama.cpp what it has, and measures what it costs. Cheap enough to
/// run on every config load.
pub async fn cache_list() -> Result<Vec<CachedModel>, String> {
    let output = tokio::process::Command::new(super::process::BINARY)
        .arg("--cache-list")
        .output()
        .await
        .map_err(|error| format!("{}: {error}", super::process::BINARY))?;

    // llama-server prints the list on stdout, but has been known to put
    // diagnostics on stderr; read both rather than depend on which.
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let mut entries = parse_cache_list(&text);
    measure(&mut entries);

    Ok(entries)
}

/// The files a repo holds, with their sizes — what the confirmation
/// prompt is asking the user to agree to.
pub async fn tree(repo: &str) -> Result<Vec<RepoFile>, String> {
    let url = format!("https://huggingface.co/api/models/{repo}/tree/main");

    let response = super::api::client()
        .get(&url)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| format!("could not reach huggingface.co: {}", chain(&error)))?;

    if !response.status().is_success() {
        return Err(format!("{url} -> HTTP {}", response.status()));
    }

    response
        .json::<Vec<RepoFile>>()
        .await
        .map_err(|error| format!("unexpected tree listing: {error}"))
}

/// Flattens an error and its causes. reqwest's own Display stops at
/// "error sending request", which never says whether the problem was DNS,
/// TLS or a refused connection.
fn chain(error: &dyn std::error::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(cause) = source {
        parts.push(cause.to_string());
        source = cause.source();
    }
    parts.join(": ")
}

/// One file in a repo, as the tree API reports it.
#[derive(Debug, Clone, Deserialize)]
pub struct RepoFile {
    pub path: String,
    /// `"file"` or `"directory"`. Load-bearing: `unsloth/gemma-4-*-GGUF`
    /// contains a directory literally named `MTP`, which a name-based test
    /// happily mistakes for the MTP head and hands to the downloader as a
    /// zero-byte artifact.
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub oid: String,
    #[serde(default)]
    pub lfs: Option<Lfs>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Lfs {
    #[serde(default)]
    pub oid: String,
}

impl RepoFile {
    /// The blob this file is stored as in the hub cache. Large files are
    /// LFS and keyed by their sha256; small ones by their git oid.
    pub fn blob(&self) -> &str {
        match &self.lfs {
            Some(lfs) if !lfs.oid.is_empty() => &lfs.oid,
            _ => &self.oid,
        }
    }

    fn stem(&self) -> &str {
        self.path.strip_suffix(".gguf").unwrap_or(&self.path)
    }

    /// A directory entry, which is never something to download.
    fn is_dir(&self) -> bool {
        self.kind.eq_ignore_ascii_case("directory")
    }

    fn is_mmproj(&self) -> bool {
        !self.is_dir() && looks_like_mmproj(&self.path)
    }

    fn is_mtp(&self) -> bool {
        !self.is_dir() && looks_like_mtp(&self.path)
    }
}

/// What a preset needs beyond its weights.
///
/// Both are read from the preset rather than assumed: a preset that says
/// `no-mmproj = true` does not want the vision projector, and only a
/// `spec-type` in the `draft-mtp` family uses the MTP head. Fetching them
/// regardless would mean hundreds of megabytes the user did not ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wants {
    pub mmproj: bool,
    pub mtp: bool,
}

impl Default for Wants {
    fn default() -> Self {
        Self {
            mmproj: true,
            mtp: false,
        }
    }
}

/// The files to fetch for one preset, in the order they are shown.
pub fn select(files: &[RepoFile], tag: Option<&str>, wants: Wants) -> Vec<RepoFile> {
    let mut chosen = weights(files, tag);

    if wants.mmproj {
        chosen.extend(pick_one(files.iter().filter(|f| f.is_mmproj())));
    }
    if wants.mtp {
        chosen.extend(pick_one(files.iter().filter(|f| f.is_mtp())));
    }

    chosen
}

/// One projector, one MTP head — not every variant a repo happens to ship.
///
/// `unsloth/gemma-4-12B-it-qat-GGUF` carries both `mmproj-BF16.gguf` and
/// `mmproj-F16.gguf`; taking both would fetch 175 MB nobody asked for.
/// BF16 wins where it exists — it is what these repos treat as the default
/// and what is already in the cache on this machine — and otherwise the
/// smallest does, since the larger variants are higher-precision copies of
/// the same thing rather than different capabilities.
fn pick_one<'a>(candidates: impl Iterator<Item = &'a RepoFile>) -> Option<RepoFile> {
    let files: Vec<&RepoFile> = candidates.collect();

    files
        .iter()
        .find(|f| f.path.to_ascii_uppercase().contains("BF16"))
        .or_else(|| files.iter().min_by_key(|f| f.size))
        .map(|f| (*f).clone())
}

/// The weight files matching a quant tag.
///
/// Ambiguity is real here: a repo can hold both `X-Q4_K_M.gguf` and
/// `X-UD-Q4_K_M.gguf`, and a naive "contains the tag" test matches both.
/// The shortest match wins, which is the one whose name carries no extra
/// qualifier beyond the tag asked for.
fn weights(files: &[RepoFile], tag: Option<&str>) -> Vec<RepoFile> {
    let Some(needle) = tag_needle(tag) else {
        return Vec::new();
    };

    let candidates: Vec<(String, RepoFile)> = files
        .iter()
        .filter(|f| !f.is_dir() && is_weight_file(&f.path))
        .filter(|f| matches_tag(f.stem(), &needle))
        .map(|f| (f.stem().to_string(), f.clone()))
        .collect();

    keep_shortest_base(candidates, &needle)
}

/// The tag as it appears inside a file name: `-ud-q4_k_xl`.
fn tag_needle(tag: Option<&str>) -> Option<String> {
    let tag = tag?.trim();
    (!tag.is_empty()).then(|| format!("-{}", tag.to_ascii_lowercase()))
}

/// A weights file rather than a projector, an MTP head or a README.
///
/// Shared with [`snapshot_weights_bytes`], which has only names to go on:
/// the tree listing and the snapshot directory spell the same repo the same
/// way, so the two must judge it the same way too.
fn is_weight_file(path: &str) -> bool {
    path.ends_with(".gguf") && !looks_like_mmproj(path) && !looks_like_mtp(path)
}

fn looks_like_mmproj(path: &str) -> bool {
    path.to_ascii_lowercase().starts_with("mmproj")
}

fn looks_like_mtp(path: &str) -> bool {
    path.to_ascii_lowercase().starts_with("mtp")
}

fn stem_of(path: &str) -> &str {
    path.strip_suffix(".gguf").unwrap_or(path)
}

/// A single file ends with the tag; a split one continues
/// `-00001-of-00003` after it.
fn matches_tag(stem: &str, needle: &str) -> bool {
    let stem = stem.to_ascii_lowercase();
    stem.ends_with(needle) || is_split_part(&stem, needle)
}

/// Keeps the candidates whose base name is shortest — the build with no
/// extra qualifier beyond the tag asked for — and every part of it, since
/// the parts of a split file share one base name.
fn keep_shortest_base<T>(candidates: Vec<(String, T)>, needle: &str) -> Vec<T> {
    let shortest = candidates
        .iter()
        .map(|(stem, _)| base_name(stem, needle).len())
        .min();

    match shortest {
        None => Vec::new(),
        Some(len) => candidates
            .into_iter()
            .filter(|(stem, _)| base_name(stem, needle).len() == len)
            .map(|(_, value)| value)
            .collect(),
    }
}

/// The part of a stem up to and including the tag, so every part of a
/// split file reduces to the same string.
fn base_name<'a>(stem: &'a str, needle: &str) -> &'a str {
    let lower = stem.to_ascii_lowercase();
    match lower.find(needle) {
        Some(at) => &stem[..at + needle.len()],
        None => stem,
    }
}

/// `...-q4_k_xl-00002-of-00003`
fn is_split_part(stem: &str, needle: &str) -> bool {
    let Some(at) = stem.find(needle) else {
        return false;
    };
    let rest = &stem[at + needle.len()..];
    let mut parts = rest.trim_start_matches('-').split('-');

    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(a), Some("of"), Some(b), None)
            if a.len() == 5 && b.len() == 5
                && a.chars().all(|c| c.is_ascii_digit())
                && b.chars().all(|c| c.is_ascii_digit())
    )
}

/// argv for the `hf` CLI: fetch exactly the chosen files, nothing else.
///
/// Each file is named outright rather than passed as a glob, so a repo
/// holding a dozen quantisations cannot surprise anyone with the wrong
/// twenty gigabytes.
///
/// They go in as the **positional `FILENAMES`**, not as `--include`, and
/// that distinction is load-bearing. `hf` used to parse its arguments with
/// argparse, where `--include` took a whole list, so `--include a b c` named
/// three files. It parses them with click now, where `--include TEXT` takes
/// exactly one value — so the same argv reads as *one* include pattern plus
/// two positional filenames, and `hf` resolves that conflict by announcing
/// "Ignoring `--include` since filenames have been explicitly set" and
/// fetching only the positional ones. For the ordinary weights + projector
/// case that silently skips the weights and still exits 0: the download
/// "succeeds", the model is not there, and the row stays "not local". Every
/// split model fails the same way, losing its first part.
///
/// Positional filenames are exact paths rather than globs and are accepted
/// by both the old and the new CLI, so this is the form that says what it
/// means on either.
pub fn download_args(repo: &str, files: &[RepoFile]) -> Vec<String> {
    let mut args = vec!["download".to_string(), repo.to_string()];
    args.extend(files.iter().map(|f| f.path.clone()));

    // `--format human` is what keeps `hf` talking at all when its output
    // is a pipe rather than a terminal; without it the command runs
    // silently and the logs panel shows nothing until it finishes.
    args.push("--format".to_string());
    args.push("human".to_string());

    args
}

/// Root of the HuggingFace hub cache, honouring `HF_HOME` the same way
/// the `hf` CLI does.
pub fn hub_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HF_HOME") {
        return Some(PathBuf::from(home).join("hub"));
    }
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".cache")
            .join("huggingface")
            .join("hub")
    })
}

/// `unsloth/Qwen3-14B-GGUF` -> `<hub>/models--unsloth--Qwen3-14B-GGUF`
pub fn repo_dir(hub: &Path, repo: &str) -> PathBuf {
    hub.join(format!("models--{}", repo.replace('/', "--")))
}

/// Bytes of `wanted` already on disk, counting a download in flight.
///
/// Measured rather than parsed, because `hf` reports only how many *files*
/// it has finished — useless for a bar when one file is 6.7 GB of a 7.1 GB
/// total. A completed file is a blob named after its hash; one still
/// arriving is `<hash>.<something>.incomplete` and grows as it lands, so
/// summing both gives a byte count that moves smoothly and finishes exactly
/// at the total.
pub fn downloaded_bytes(repo_dir: &Path, wanted: &[RepoFile]) -> u64 {
    let blobs = repo_dir.join("blobs");

    let complete: u64 = wanted
        .iter()
        .filter(|file| blobs.join(file.blob()).is_file())
        .map(|file| file.size)
        .sum();

    complete + partial_bytes(repo_dir)
}

/// Suffixes the two downloaders use for a file still arriving.
///
/// **Both**, because either can be the one fetching: `hf` when the user
/// asks herd to download, and `llama-server` itself when a launch finds
/// the weights absent. Counting only `hf`'s meant a launch-time download —
/// the slow, 16 GiB case people actually sit and watch — showed no
/// progress at all, and then timed out as a failure to bind.
const PARTIAL_SUFFIXES: [&str; 2] = [".incomplete", ".downloadInProgress"];

/// Bytes sitting in half-finished blobs for this repo, whoever is writing
/// them. Zero when the directory does not exist, which is the normal state
/// before anything starts.
pub fn partial_bytes(repo_dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(repo_dir.join("blobs")) else {
        return 0;
    };

    entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            PARTIAL_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
        })
        .filter_map(|entry| entry.metadata().ok())
        .map(|meta| meta.len())
        .sum()
}

/// [`partial_bytes`] for an ini `hf-repo` reference, which is what the
/// launch path has to hand rather than a directory.
pub fn partial_bytes_for(reference: &str) -> u64 {
    let Some(hub) = hub_dir() else {
        return 0;
    };
    partial_bytes(&repo_dir(&hub, split_repo(reference).0))
}

/// Human-readable size, for a prompt that is asking someone to commit to
/// a download.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("G", 1 << 30), ("M", 1 << 20), ("K", 1 << 10)];

    for (suffix, scale) in UNITS {
        if bytes >= scale {
            return format!("{:.1}{suffix}", bytes as f64 / scale as f64);
        }
    }
    format!("{bytes}B")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact shape `llama-server --cache-list` prints on this machine.
    const CACHE_LIST: &str = "\
number of models in cache: 3
   1. unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL
   2. huihui-ai/Huihui-Qwen3.6-35B-A3B-Claude-4.7-Opus-abliterated-MTP-GGUF:Q4_K
   3. unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_XL
";

    #[test]
    fn the_cache_list_header_is_not_mistaken_for_a_model() {
        let cached = parse_cache_list(CACHE_LIST);

        assert_eq!(cached.len(), 3);
        assert_eq!(
            cached[0].reference,
            "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL"
        );
        assert!(!cached
            .iter()
            .any(|entry| entry.reference.contains("number of")));
        // A parse claims nothing about disk: measuring is a separate step,
        // and an unmeasured entry must not read as an empty one.
        assert!(cached.iter().all(|entry| entry.bytes.is_none()));
    }

    /// Two quantisations of one repo share a blobs directory, so they share
    /// a size — which the UI has to be able to say rather than implying the
    /// disk holds it twice.
    #[test]
    fn entries_from_the_same_repo_know_they_share_a_directory() {
        let cached = parse_cache_list(CACHE_LIST);
        let other = CachedModel::from("unsloth/gemma-4-12B-it-qat-GGUF:Q8_0");

        assert!(cached[0].same_repo(&other));
        assert!(!cached[0].same_repo(&cached[1]));
    }

    #[test]
    fn junk_never_becomes_a_phantom_cached_model() {
        assert!(parse_cache_list("").is_empty());
        assert!(parse_cache_list("error: something went wrong").is_empty());
        assert!(parse_cache_list("  1. not-a-repo").is_empty());
    }

    /// The ini and the cache spell the same model differently: llama.cpp
    /// drops Unsloth's `UD-` prefix from the quant tag.
    #[test]
    fn a_preset_matches_its_cache_entry_despite_the_ud_prefix() {
        let cached = parse_cache_list(CACHE_LIST);

        assert_eq!(
            availability("unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL", &cached),
            Availability::Local
        );
    }

    /// A scratch directory of this test's own. The thread id is in the
    /// name as well as the pid because tests run in parallel, and two of
    /// them sharing a fixture directory is a failure that moves about
    /// between runs.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "herd-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// The blobs are the weights; the snapshot directory beside them is
    /// symlinks into the same bytes, so a naive walk of the repo would
    /// report every model twice.
    #[test]
    fn a_repo_is_measured_by_its_blobs_and_nothing_else() {
        let dir = scratch("size");
        let blobs = dir.join("blobs");
        std::fs::create_dir_all(&blobs).expect("blobs dir");
        std::fs::create_dir_all(dir.join("snapshots/main")).expect("snapshot dir");
        std::fs::write(blobs.join("sha-weights"), vec![0; 1_000]).expect("blob");
        std::fs::write(dir.join("snapshots/main/weights.gguf"), vec![0; 1_000]).expect("snapshot");

        assert_eq!(repo_bytes(&dir), Some(1_000));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A repo that is not on this machine has no size — which is not the
    /// same as a size of zero, and must not be printed as one.
    #[test]
    fn an_unreadable_repo_directory_has_no_size_rather_than_zero() {
        assert_eq!(repo_bytes(Path::new("/nonexistent/herd/repo")), None);
    }

    /// A repo cache with a stale revision left behind, which is what this
    /// machine's `gemma-4-12B-it-qat-GGUF` actually looks like.
    #[cfg(unix)]
    fn fake_repo(name: &str) -> PathBuf {
        use std::os::unix::fs::symlink;

        let dir = scratch(name);
        let blobs = dir.join("blobs");
        std::fs::create_dir_all(&blobs).expect("blobs");
        std::fs::create_dir_all(dir.join("refs")).expect("refs");
        std::fs::write(dir.join("refs/main"), "rev-current\n").expect("ref");

        for (blob, size) in [
            ("sha-current", 7_000),
            ("sha-mmproj", 300),
            ("sha-stale", 6_000),
        ] {
            std::fs::write(blobs.join(blob), vec![0; size]).expect("blob");
        }

        for (revision, weights) in [("rev-current", "sha-current"), ("rev-stale", "sha-stale")] {
            let snapshot = dir.join("snapshots").join(revision);
            std::fs::create_dir_all(&snapshot).expect("snapshot");
            symlink(
                blobs.join(weights),
                snapshot.join("Model-9B-UD-Q4_K_XL.gguf"),
            )
            .expect("weights link");
            symlink(blobs.join("sha-mmproj"), snapshot.join("mmproj-BF16.gguf"))
                .expect("mmproj link");
        }

        dir
    }

    /// The number the Models screen shows is the file llama.cpp would
    /// open: the current revision's weights, and only those.
    ///
    /// The mistake this guards against is summing the blobs instead. A repo
    /// keeps every revision it has ever fetched, so that would announce a
    /// model at twice its size — and the projector beside it is not weights
    /// either.
    #[cfg(unix)]
    #[test]
    fn a_model_is_measured_from_the_current_revision_only() {
        let dir = fake_repo("weights");

        assert_eq!(
            snapshot_weights_bytes(&dir, Some("UD-Q4_K_XL")),
            Some(7_000)
        );
        // ...against the disk the whole repo occupies, which is the other
        // question and the larger number.
        assert_eq!(repo_bytes(&dir), Some(13_300));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A quantisation the repo does not hold has no size here, even though
    /// the repo is plainly present. Falling back to "some other quant's
    /// file" would be a confident, specific, wrong number.
    #[cfg(unix)]
    #[test]
    fn a_quantisation_that_is_not_cached_is_not_measured() {
        let dir = fake_repo("other-quant");

        assert_eq!(snapshot_weights_bytes(&dir, Some("Q8_0")), None);
        assert_eq!(snapshot_weights_bytes(&dir, None), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same measurement, reached the way the UI reaches it: by the ini
    /// reference, against the cache list.
    #[test]
    fn a_measured_entry_is_found_by_repo_and_quantisation() {
        let cached = vec![CachedModel {
            weights: Some(7_000),
            bytes: Some(13_300),
            ..CachedModel::from("unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL")
        }];

        // The ini spells it `UD-Q4_K_XL`, the cache `Q4_K_XL`.
        assert_eq!(
            measured_weights("unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL", &cached),
            Some(7_000)
        );
        // A different quantisation of the same repo is a different file,
        // and this one has not been measured.
        assert_eq!(
            measured_weights("unsloth/gemma-4-12B-it-qat-GGUF:Q8_0", &cached),
            None
        );
        assert_eq!(
            measured_weights("unsloth/Qwen3-14B-GGUF:Q4_K_XL", &cached),
            None
        );
    }

    /// `availability` and `measured_weights` deliberately disagree about a
    /// repo cached under another tag: the first says "launching will not
    /// download anything", the second refuses to size a file it has not
    /// seen. Both are right about their own question.
    #[test]
    fn a_repo_under_another_tag_is_available_but_not_measured() {
        let cached = vec![CachedModel {
            weights: Some(7_000),
            ..CachedModel::from("unsloth/Qwen3-14B-GGUF:Q4_K_XL")
        }];
        let asked = "unsloth/Qwen3-14B-GGUF:Q8_0";

        assert_eq!(availability(asked, &cached), Availability::Local);
        assert_eq!(measured_weights(asked, &cached), None);
    }

    #[test]
    fn a_repo_that_is_not_cached_is_missing() {
        let cached = parse_cache_list(CACHE_LIST);

        assert_eq!(
            availability("unsloth/Qwen3.5-9B-GGUF:UD-Q4_K_XL", &cached),
            Availability::Missing
        );
    }

    /// Nothing cached at all is a real answer, not an unknown one: an
    /// empty cache genuinely means everything needs downloading.
    #[test]
    fn an_empty_cache_reports_everything_missing() {
        assert_eq!(
            availability("unsloth/Qwen3-14B-GGUF:UD-Q4_K_XL", &[]),
            Availability::Missing
        );
    }

    fn file(path: &str, size: u64) -> RepoFile {
        RepoFile {
            path: path.to_string(),
            kind: "file".to_string(),
            size,
            oid: format!("oid-{path}"),
            lfs: Some(Lfs {
                oid: format!("sha-{path}"),
            }),
        }
    }

    fn repo() -> Vec<RepoFile> {
        vec![
            file(".gitattributes", 1_438),
            file("README.md", 19_787),
            file("Qwen3.5-9B-Q4_K_M.gguf", 5_680_000_000),
            file("Qwen3.5-9B-UD-Q4_K_XL.gguf", 5_390_000_000),
            file("Qwen3.5-9B-UD-Q4_K_M.gguf", 5_700_000_000),
            file("mmproj-BF16.gguf", 254_000_000),
            file("mtp-Qwen3.5-9B.gguf", 175_000_000),
        ]
    }

    #[test]
    fn the_quant_tag_picks_exactly_one_set_of_weights() {
        let chosen = select(
            &repo(),
            Some("UD-Q4_K_XL"),
            Wants {
                mmproj: false,
                mtp: false,
            },
        );

        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].path, "Qwen3.5-9B-UD-Q4_K_XL.gguf");
    }

    /// The ambiguity that a "contains the tag" test gets wrong: `Q4_K_M`
    /// must not also pull `UD-Q4_K_M`, which is a different quantisation
    /// and another 5.7 GB.
    #[test]
    fn a_tag_does_not_match_a_longer_qualified_one() {
        let chosen = select(
            &repo(),
            Some("Q4_K_M"),
            Wants {
                mmproj: false,
                mtp: false,
            },
        );

        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].path, "Qwen3.5-9B-Q4_K_M.gguf");
    }

    /// The three artifact kinds, each pulled only when the preset says it
    /// wants it.
    #[test]
    fn mmproj_and_mtp_are_fetched_only_when_wanted() {
        let all = select(
            &repo(),
            Some("UD-Q4_K_XL"),
            Wants {
                mmproj: true,
                mtp: true,
            },
        );
        let paths: Vec<&str> = all.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "Qwen3.5-9B-UD-Q4_K_XL.gguf",
                "mmproj-BF16.gguf",
                "mtp-Qwen3.5-9B.gguf"
            ]
        );

        let weights_only = select(
            &repo(),
            Some("UD-Q4_K_XL"),
            Wants {
                mmproj: false,
                mtp: false,
            },
        );
        assert_eq!(weights_only.len(), 1);
    }

    /// A split model must be fetched whole; taking one part of three would
    /// leave an unloadable model behind.
    #[test]
    fn every_part_of_a_split_model_is_selected() {
        let split = vec![
            file("Big-UD-Q4_K_XL-00001-of-00003.gguf", 10),
            file("Big-UD-Q4_K_XL-00002-of-00003.gguf", 10),
            file("Big-UD-Q4_K_XL-00003-of-00003.gguf", 10),
            file("Big-Q8_0.gguf", 99),
        ];
        let chosen = select(
            &split,
            Some("UD-Q4_K_XL"),
            Wants {
                mmproj: false,
                mtp: false,
            },
        );

        assert_eq!(chosen.len(), 3, "a split model was fetched in pieces");
    }

    /// Regression, straight from `unsloth/gemma-4-12B-it-qat-GGUF`: the
    /// repo contains a *directory* literally named `MTP`. A name-based
    /// test matched it, and it went to the downloader as a zero-byte
    /// artifact with a git oid for a blob.
    #[test]
    fn a_directory_is_never_mistaken_for_an_artifact() {
        let listing = vec![
            RepoFile {
                path: "MTP".to_string(),
                kind: "directory".to_string(),
                size: 0,
                oid: "1abdf4a4f7bd6c5f29eb41fd7ee2ca0f2713bb50".to_string(),
                lfs: None,
            },
            file("mtp-gemma-4-12B-it.gguf", 253_708_800),
            file("gemma-4-12B-it-qat-UD-Q4_K_XL.gguf", 6_716_356_800),
        ];

        let chosen = select(
            &listing,
            Some("UD-Q4_K_XL"),
            Wants {
                mmproj: false,
                mtp: true,
            },
        );

        assert!(
            chosen.iter().all(|f| f.path != "MTP"),
            "a directory was selected for download"
        );
        assert!(chosen.iter().all(|f| f.size > 0), "a zero-byte artifact");
        assert_eq!(chosen.len(), 2);
    }

    /// The same repo ships `mmproj-BF16.gguf` *and* `mmproj-F16.gguf`.
    /// Taking both is 175 MB nobody asked for.
    #[test]
    fn only_one_projector_is_fetched() {
        let listing = vec![
            file("mmproj-F16.gguf", 175_115_840),
            file("mmproj-BF16.gguf", 175_115_840),
            file("model-UD-Q4_K_XL.gguf", 100),
        ];

        let chosen = select(
            &listing,
            Some("UD-Q4_K_XL"),
            Wants {
                mmproj: true,
                mtp: false,
            },
        );

        let projectors: Vec<&str> = chosen
            .iter()
            .filter(|f| f.path.starts_with("mmproj"))
            .map(|f| f.path.as_str())
            .collect();
        assert_eq!(projectors, vec!["mmproj-BF16.gguf"]);
    }

    /// Without a tag there is no way to know which quantisation is meant,
    /// and guessing would download the wrong several gigabytes.
    #[test]
    fn no_tag_selects_no_weights() {
        let chosen = select(&repo(), None, Wants::default());
        assert!(chosen.iter().all(|f| f.is_mmproj()));
    }

    #[test]
    fn split_repo_separates_the_quant_tag() {
        assert_eq!(
            split_repo("unsloth/Qwen3-14B-GGUF:UD-Q4_K_XL"),
            ("unsloth/Qwen3-14B-GGUF", Some("UD-Q4_K_XL"))
        );
        assert_eq!(
            split_repo("unsloth/Qwen3-14B-GGUF"),
            ("unsloth/Qwen3-14B-GGUF", None)
        );
    }

    /// Files are named outright, never globbed: a repo holding a dozen
    /// quantisations must not be able to surprise anyone.
    #[test]
    fn the_argv_names_every_file_and_keeps_hf_talking() {
        let files = select(
            &repo(),
            Some("UD-Q4_K_XL"),
            Wants {
                mmproj: true,
                mtp: false,
            },
        );
        let args = download_args("unsloth/Qwen3.5-9B-GGUF", &files);

        assert_eq!(
            args,
            [
                "download",
                "unsloth/Qwen3.5-9B-GGUF",
                "Qwen3.5-9B-UD-Q4_K_XL.gguf",
                "mmproj-BF16.gguf",
                "--format",
                "human",
            ]
        );
        assert!(!args.iter().any(|arg| arg.contains('*')), "{args:?}");
    }

    /// Regression, and the reason downloads failed on a machine with
    /// `hf` 1.27: under click, `--include` takes exactly one value, so
    /// `--include weights.gguf mmproj.gguf` parses as one glob plus one
    /// positional filename — and `hf` then ignores the `--include`
    /// outright and fetches the projector *without the weights*, exiting
    /// 0. Naming the files positionally is the only form that means the
    /// same thing to both the old and the new CLI.
    #[test]
    fn the_files_are_positional_so_hf_cannot_drop_the_weights() {
        let files = select(
            &repo(),
            Some("UD-Q4_K_XL"),
            Wants {
                mmproj: true,
                mtp: true,
            },
        );
        let args = download_args("unsloth/Qwen3.5-9B-GGUF", &files);

        assert!(
            !args.iter().any(|arg| arg == "--include"),
            "--include silently drops all but one file under hf >= 1.27: {args:?}"
        );
        // Every selected file survives into the argv, in order, straight
        // after the repo — nothing may be left behind by the flag parser.
        let named: Vec<&str> = args[2..args.len() - 2].iter().map(String::as_str).collect();
        let wanted: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(named, wanted);
        // Without this `hf` prints nothing at all into a pipe.
        assert_eq!(args[args.len() - 2..], ["--format", "human"]);
    }

    #[test]
    fn repo_dir_uses_the_hub_naming() {
        let dir = repo_dir(Path::new("/hub"), "unsloth/Qwen3-14B-GGUF");
        assert!(dir.ends_with("models--unsloth--Qwen3-14B-GGUF"));
    }

    #[test]
    fn human_bytes_reads_at_a_glance() {
        assert_eq!(human_bytes(6_700_000_000), "6.2G");
        assert_eq!(human_bytes(254_000_000), "242.2M");
        assert_eq!(human_bytes(512), "512B");
    }

    /// Progress counts a finished blob and a partial one together, so the
    /// bar moves during the download and lands exactly on the total.
    #[test]
    fn progress_counts_finished_and_in_flight_bytes() {
        let dir = std::env::temp_dir().join(format!("herd-hub-{}", std::process::id()));
        let blobs = dir.join("blobs");
        std::fs::create_dir_all(&blobs).expect("blobs dir");

        let wanted = vec![file("weights.gguf", 1_000), file("mmproj-BF16.gguf", 500)];

        // Nothing there yet.
        assert_eq!(downloaded_bytes(&dir, &wanted), 0);

        // The mmproj has landed; the weights are still arriving.
        std::fs::write(blobs.join("sha-mmproj-BF16.gguf"), b"x").expect("write blob");
        std::fs::write(
            blobs.join("sha-weights.gguf.abc123.incomplete"),
            vec![0; 400],
        )
        .expect("write partial");
        assert_eq!(downloaded_bytes(&dir, &wanted), 500 + 400);

        // Both complete: exactly the total, so the bar can reach 100%.
        std::fs::remove_file(blobs.join("sha-weights.gguf.abc123.incomplete")).expect("rm");
        std::fs::write(blobs.join("sha-weights.gguf"), b"x").expect("write blob");
        assert_eq!(downloaded_bytes(&dir, &wanted), 1_500);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A repo directory that does not exist yet is simply zero bytes in,
    /// not an error: it is the normal state before a download starts.
    #[test]
    fn a_missing_repo_directory_is_zero_not_a_failure() {
        let wanted = vec![file("weights.gguf", 1_000)];
        assert_eq!(
            downloaded_bytes(Path::new("/nonexistent/herd/repo"), &wanted),
            0
        );
    }

    /// Checks against the real world, which is where every assumption in
    /// this module came from. Ignored by default — they need
    /// `llama-server` on the PATH and, for the tree, network. Run with:
    ///   cargo test -- --ignored --test-threads=1
    mod live {
        use super::*;

        #[tokio::test]
        #[ignore = "requires llama-server on the PATH"]
        async fn the_cache_list_parses_against_a_real_llama_server() {
            let cached = cache_list().await.expect("llama-server --cache-list");

            // An empty cache is legitimate, but every entry must look like
            // a repo reference rather than a stray line of output.
            for entry in &cached {
                let reference = &entry.reference;
                assert!(reference.contains('/'), "not a repo reference: {entry:?}");
                assert!(!reference.contains("number of"), "header leaked: {entry:?}");
            }
        }

        /// Sizes, against the cache this machine really has.
        ///
        /// The two figures must both be there and must not be equal for
        /// every entry: `weights` is one revision's gguf and `bytes` is
        /// everything the repo has ever kept, and this machine's
        /// `gemma-4-12B-it-qat-GGUF` holds two 6.7 GiB copies of the same
        /// file. A run where they always matched would mean the snapshot
        /// resolution had silently fallen back to summing blobs.
        #[tokio::test]
        #[ignore = "requires llama-server on the PATH and a populated cache"]
        async fn the_cache_is_measured_per_model_and_per_repo() {
            let cached = cache_list().await.expect("llama-server --cache-list");

            for entry in &cached {
                let (Some(weights), Some(disk)) = (entry.weights, entry.bytes) else {
                    panic!("unmeasured cache entry: {entry:?}");
                };
                assert!(weights > 0, "a listed model with no weights: {entry:?}");
                assert!(
                    weights <= disk,
                    "{} claims {weights} bytes of weights in a {disk}-byte repo",
                    entry.reference
                );
            }
        }

        /// The shipped tiers are real presets, so every one of them must
        /// resolve to a definite answer against the real cache — never a
        /// panic, and never `Unknown`, which is reserved for "could not
        /// ask at all".
        #[tokio::test]
        #[ignore = "requires llama-server on the PATH"]
        async fn every_shipped_preset_gets_a_verdict() {
            let cached = cache_list().await.expect("llama-server --cache-list");

            for tier in ["16gb", "32gb"] {
                let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("data")
                    .join(tier)
                    .join("models.ini");
                let config = crate::services::llama::load(&path).expect("shipped tier");

                for name in config.model_names() {
                    let Some(repo) = config
                        .model(name)
                        .and_then(|section| section.get("hf-repo"))
                        .map(str::to_string)
                    else {
                        continue;
                    };
                    assert_ne!(
                        availability(&repo, &cached),
                        Availability::Unknown,
                        "{tier}/{name} got no verdict"
                    );
                }
            }
        }

        /// The path derivation is the risky half of the download phase:
        /// if `partial_bytes_for` looks in the wrong directory it silently
        /// returns zero, and a launch-time download reads as a failure to
        /// bind again. Checked against whatever partials the real cache
        /// happens to hold.
        #[tokio::test]
        #[ignore = "reads the real HuggingFace cache"]
        async fn partials_are_found_in_the_real_cache() {
            let Some(hub) = hub_dir() else {
                eprintln!("skipping: no HOME");
                return;
            };
            assert!(hub.is_dir(), "hub cache not at {}", hub.display());

            // Every repo directory the cache holds, and what we make of it.
            let mut seen = 0;
            for entry in std::fs::read_dir(&hub).expect("read hub").flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let Some(repo) = name.strip_prefix("models--") else {
                    continue;
                };
                let reference = repo.replace("--", "/");

                let direct = partial_bytes(&entry.path());
                let derived = partial_bytes_for(&reference);
                assert_eq!(
                    direct, derived,
                    "{reference}: the reference form found {derived} where the \
                     directory form found {direct}"
                );

                if direct > 0 {
                    eprintln!("  {reference}: {} in partials", human_bytes(direct));
                    seen += 1;
                }
            }
            eprintln!("  {seen} repo(s) with a download in flight");
        }

        /// The argv is checked against the real `hf`, because every other
        /// test here only proves we *built* the arguments we meant to —
        /// not that the CLI reads them the same way. That gap is precisely
        /// where downloads broke: `--include a b c` named three files under
        /// argparse and names one glob plus two positional filenames under
        /// click, so `hf` quietly fetched the projector without the weights
        /// and exited 0.
        ///
        /// `--dry-run` makes this free: it resolves and prints what would
        /// be fetched without transferring anything, so the assertion is
        /// that `hf` agrees on the *count* of files we asked for.
        #[tokio::test]
        #[ignore = "requires the hf CLI and network access to huggingface.co"]
        async fn hf_resolves_every_file_the_argv_names() {
            let reference = "unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL";
            let (repo, tag) = split_repo(reference);

            let files = tree(repo).await.expect("tree listing");
            let chosen = select(
                &files,
                tag,
                Wants {
                    mmproj: true,
                    mtp: true,
                },
            );
            assert!(chosen.len() > 1, "need >1 file to exercise the bug");

            let mut args = download_args(repo, &chosen);
            args.push("--dry-run".to_string());

            let output = tokio::process::Command::new(DOWNLOADER)
                .args(&args)
                .output()
                .await
                .expect("run hf");
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );

            assert!(output.status.success(), "hf refused the argv:\n{text}");
            // The tell-tale from the broken form. `hf` emits it and carries
            // on with a subset, so it must be treated as a failure here.
            assert!(
                !text.contains("Ignoring `--include`"),
                "hf discarded part of the argv:\n{text}"
            );
            // Every file we named must appear in what hf resolved.
            for file in &chosen {
                assert!(
                    text.contains(&file.path),
                    "hf did not resolve {}:\n{text}",
                    file.path
                );
            }
            // ...and the count it resolved must be the count we named.
            //
            // Two forms, because `hf` phrases it differently depending on
            // what is already here: "Will download 3 files" on a cold
            // cache, "Will download 0 files (out of 3)" once they are all
            // present. Asserting only the first made this test pass or fail
            // on whether the developer happened to have the model — which
            // is not what it is here to check.
            let count = chosen.len();
            assert!(
                text.contains(&format!("{count} files"))
                    || text.contains(&format!("(out of {count})")),
                "hf resolved a different number of files than the {count} named:\n{text}"
            );
        }

        /// The tree API is what the confirmation prompt's sizes come from,
        /// and what the file selection runs against.
        #[tokio::test]
        #[ignore = "requires network access to huggingface.co"]
        async fn a_real_repo_listing_selects_the_three_artifacts() {
            let files = tree("unsloth/gemma-4-12B-it-qat-GGUF")
                .await
                .expect("tree listing");

            let chosen = select(
                &files,
                Some("UD-Q4_K_XL"),
                Wants {
                    mmproj: true,
                    mtp: true,
                },
            );
            let paths: Vec<&str> = chosen.iter().map(|f| f.path.as_str()).collect();

            assert!(
                paths.iter().any(|p| p.contains("UD-Q4_K_XL")),
                "no weights selected: {paths:?}"
            );
            assert!(
                paths.iter().any(|p| p.starts_with("mmproj")),
                "no mmproj: {paths:?}"
            );
            assert!(
                paths.iter().any(|p| p.starts_with("mtp")),
                "no mtp head: {paths:?}"
            );
            for file in &chosen {
                assert!(
                    file.size > 0 && !file.blob().is_empty(),
                    "{} has size {} and blob {:?}",
                    file.path,
                    file.size,
                    file.blob()
                );
            }
        }
    }
}
