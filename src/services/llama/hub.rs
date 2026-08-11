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

/// Reads `llama-server --cache-list` output into `repo:quant` entries.
///
/// The first line is a count ("number of models in cache: 10") and the
/// rest are numbered. Anything unparseable is skipped rather than guessed
/// at — a mangled line must not become a phantom cached model.
pub fn parse_cache_list(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (index, rest) = line.split_once('.')?;
            index.trim().parse::<u32>().ok()?;
            let entry = rest.trim();
            (!entry.is_empty() && entry.contains('/')).then(|| entry.to_string())
        })
        .collect()
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
pub fn availability(reference: &str, cached: &[String]) -> Availability {
    let (repo, tag) = split_repo(reference);

    let same_repo = |entry: &String| {
        let (cached_repo, _) = split_repo(entry);
        cached_repo.eq_ignore_ascii_case(repo)
    };

    match (cached.iter().any(same_repo), tag) {
        (false, _) => Availability::Missing,
        (true, _) => Availability::Local,
    }
}

/// Asks llama.cpp what it has. Cheap enough to run on every config load.
pub async fn cache_list() -> Result<Vec<String>, String> {
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

    Ok(parse_cache_list(&text))
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
        !self.is_dir() && self.path.to_ascii_lowercase().starts_with("mmproj")
    }

    fn is_mtp(&self) -> bool {
        !self.is_dir() && self.path.to_ascii_lowercase().starts_with("mtp")
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
    let Some(tag) = tag else {
        return Vec::new();
    };
    let needle = format!("-{}", tag.to_ascii_lowercase());

    let candidates: Vec<&RepoFile> = files
        .iter()
        .filter(|f| !f.is_dir() && f.path.ends_with(".gguf") && !f.is_mmproj() && !f.is_mtp())
        .filter(|f| {
            let stem = f.stem().to_ascii_lowercase();
            // A single file ends with the tag; a split one continues
            // "-00001-of-00003" after it.
            stem.ends_with(&needle) || is_split_part(&stem, &needle)
        })
        .collect();

    // Split files share one base name, so keep every part of the winner.
    let shortest = candidates
        .iter()
        .map(|f| base_name(f.stem(), &needle).len())
        .min();

    match shortest {
        None => Vec::new(),
        Some(len) => candidates
            .into_iter()
            .filter(|f| base_name(f.stem(), &needle).len() == len)
            .cloned()
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
        assert_eq!(cached[0], "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL");
        assert!(!cached.iter().any(|entry| entry.contains("number of")));
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
                assert!(entry.contains('/'), "not a repo reference: {entry:?}");
                assert!(!entry.contains("number of"), "header leaked: {entry:?}");
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
            assert!(
                text.contains(&format!("{} files", chosen.len())),
                "hf resolved a different number of files than the {} named:\n{text}",
                chosen.len()
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
