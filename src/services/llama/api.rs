//! Minimal HTTP client for talking to a running llama-server instance —
//! the equivalent of `test_call.sh` (`ping`) and a `/v1/models` reachability
//! check (`status`).

use chrono::{DateTime, Local};
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::Duration;

const CHAT_TIMEOUT: Duration = Duration::from_secs(120);
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(3);
/// How many times to re-check a busy port before believing it.
const PORT_SETTLE_ATTEMPTS: usize = 3;
const PORT_SETTLE_INTERVAL: Duration = Duration::from_millis(250);

/// One client for the whole process, rather than one per request.
///
/// The health poller runs for the entire life of a launch; building a
/// fresh `Client` on every probe meant a new connection pool and resolver
/// every few hundred milliseconds, and threw away the keep-alive
/// connection each time — measurable churn on a small machine that is
/// already under memory pressure. Timeouts are per-request instead, since
/// a 3s health probe and a 120s chat completion cannot share one.
pub(crate) fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default()
    })
}

pub fn base_url(host: &str, port: u16) -> String {
    format!("http://{host}:{port}")
}

/// Outcome of one `GET /health` probe. llama-server answers 503 while a
/// model is still loading and 200 once it can serve, which is exactly the
/// STARTING -> SERVING edge — far more reliable than grepping stdout for
/// the word "listening", whose wording changes between releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// 200: ready to serve.
    Serving,
    /// Reachable but not ready yet (503 while loading, or any other status).
    Loading,
    /// Nothing listening yet, or the connection failed.
    Unreachable,
}

pub async fn health(base_url: &str) -> Health {
    match client()
        .get(format!("{base_url}/health"))
        .timeout(HEALTH_TIMEOUT)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => Health::Serving,
        Ok(_) => Health::Loading,
        Err(_) => Health::Unreachable,
    }
}

/// Best-effort check for "is something already bound to this port". Used
/// before launching so a tier switch does not fail with an opaque
/// address-in-use error from llama-server.
///
/// A successful TCP connect is the signal; herd never kills whatever it
/// finds, since it has no way to know the process is one it may touch.
pub async fn port_in_use(host: &str, port: u16) -> bool {
    let target = format!("{host}:{port}");
    let connect = tokio::net::TcpStream::connect(target);

    matches!(
        tokio::time::timeout(Duration::from_millis(300), connect).await,
        Ok(Ok(_))
    )
}

/// [`port_in_use`], but gives a port that is *being* released a moment to
/// finish. A server killed a second ago can still accept a connection
/// while the kernel tears its socket down, which would otherwise raise a
/// "port in use" prompt against a process that no longer exists.
pub async fn port_in_use_settled(host: &str, port: u16) -> bool {
    for attempt in 0..PORT_SETTLE_ATTEMPTS {
        if !port_in_use(host, port).await {
            return false;
        }
        if attempt + 1 < PORT_SETTLE_ATTEMPTS {
            tokio::time::sleep(PORT_SETTLE_INTERVAL).await;
        }
    }
    true
}

/// Turns a failed request into something the user can act on.
///
/// Every feature that talks to `/v1/...` needs a server to be serving, and
/// when none is, reqwest says `error sending request for url
/// (http://127.0.0.1:1234/v1/models)`. That describes the plumbing rather
/// than the situation: nothing is wrong with the request, there is simply
/// no llama-server running. A user reading it has to know that "error
/// sending request" means "connection refused" to work out that the fix is
/// to launch something.
///
/// So a refused connection is reported as what it is, with the two ways
/// out of it. Anything else keeps its detail: a timeout means something
/// *did* answer the door and is a different problem, and an unrecognised
/// failure is flattened through its source chain rather than truncated at
/// reqwest's own top-level message.
///
/// The base URL is named because it is the fact the user has to check —
/// a server on the wrong port is indistinguishable from no server at all.
fn unreachable(base_url: &str, error: &reqwest::Error) -> String {
    if error.is_connect() || refused(error) {
        return format!(
            "nothing is listening on {base_url} — no llama-server is running \
             (start one with :launch <model>, or :router)"
        );
    }
    if error.is_timeout() {
        return format!("{base_url} accepted the connection but did not answer in time");
    }

    format!("{base_url}: {}", chain(error))
}

/// Was the connection refused, whatever reqwest calls that this week?
///
/// `is_connect()` is the documented test and covers it today, but the
/// classification has moved between reqwest releases before; the io error
/// underneath does not move, so it is worth also looking there rather than
/// silently falling back to the plumbing message.
fn refused(error: &reqwest::Error) -> bool {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);

    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            return matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::AddrNotAvailable
            );
        }
        source = cause.source();
    }
    false
}

/// Flattens an error and its causes. reqwest's own Display stops at the
/// top, which never says whether the problem was DNS, TLS or the socket.
fn chain(error: &dyn std::error::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(cause) = source {
        parts.push(cause.to_string());
        source = cause.source();
    }
    parts.join(": ")
}

/// The system prompt from `test_call.sh`, kept verbatim so a probe from
/// herd and a run of the script are comparable.
pub const SYSTEM_PROMPT: &str =
    "You are a helpful assistant. Do not show reasoning. Answer directly.";
/// The script's default user message.
pub const DEFAULT_PROMPT: &str = "Bonjour";

/// A completed chat probe.
///
/// `sent_at` and `latency` are measured locally and so are always present.
/// Everything else is read opportunistically: `usage` is standard but
/// optional, and `timings` is a llama.cpp extension that other servers do
/// not send.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatOutcome {
    pub model: String,
    pub prompt: String,
    pub reply: String,
    /// Wall-clock time the request went out. Kept alongside the duration
    /// because "1.25s" answers a different question from "at 14:32:07" —
    /// the second is what lets a probe be lined up against a log line.
    pub sent_at: DateTime<Local>,
    pub latency: Duration,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub tokens_per_second: Option<f64>,
    /// Server-side split of the round trip, from llama.cpp's `timings`:
    /// how long it spent ingesting the prompt, and how long generating.
    /// Together they explain a latency that `tok/s` alone does not — a
    /// long prompt eval on a paging machine looks identical to a slow
    /// model until these are broken out.
    pub prompt_ms: Option<f64>,
    pub predicted_ms: Option<f64>,
}

#[cfg(test)]
impl ChatOutcome {
    /// A plausible outcome, so a new field on this struct does not mean
    /// editing every test in the crate that builds one. Tests override the
    /// fields they care about with `..ChatOutcome::sample()`.
    ///
    /// `sent_at` is a fixed local wall-clock time rather than `now()`, so
    /// anything asserting on the formatted timestamp is deterministic.
    pub fn sample() -> Self {
        use chrono::TimeZone;

        Self {
            model: "gemma4-12b".into(),
            prompt: DEFAULT_PROMPT.into(),
            reply: "Bonjour !".into(),
            sent_at: Local
                .with_ymd_and_hms(2026, 8, 11, 14, 32, 7)
                .earliest()
                .expect("a valid local time"),
            latency: Duration::from_millis(1250),
            prompt_tokens: Some(24),
            completion_tokens: Some(12),
            tokens_per_second: Some(9.6),
            prompt_ms: None,
            predicted_ms: None,
        }
    }
}

impl ChatOutcome {
    /// Time to the first token, as the client experienced it.
    ///
    /// **Derived, and only where the server accounts for its own
    /// generation.** The probe is deliberately non-streaming — the same
    /// request `test_call.sh` makes, so the two stay comparable — which
    /// means nothing here ever sees the first token arrive. What is known
    /// is the whole round trip, measured locally, and llama.cpp's
    /// `predicted_ms` for the generation; the difference is everything
    /// that happened *before* the first token, which is queueing, prompt
    /// ingestion and the network. That is the number a user is asking for
    /// when they ask how long until it starts answering.
    ///
    /// `None` when the server sends no `timings` — the usual restraint:
    /// there is no honest TTFT to report, so none is reported. `None` too
    /// when the subtraction goes negative, which is clock noise on a fast
    /// local request rather than a measurement.
    pub fn ttft(&self) -> Option<Duration> {
        let predicted = self
            .predicted_ms
            .filter(|ms| ms.is_finite() && *ms >= 0.0)?;

        self.latency
            .checked_sub(Duration::from_secs_f64(predicted / 1000.0))
    }

    /// One-line summary for the log panel.
    pub fn summary(&self) -> String {
        let mut parts = vec![format!("{:.2}s", self.latency.as_secs_f64())];
        if let Some(tokens) = self.completion_tokens {
            parts.push(format!("{tokens} tok"));
        }
        if let Some(rate) = self.tokens_per_second {
            parts.push(format!("{rate:.1} tok/s"));
        }
        format!("{} ({})", self.model, parts.join(", "))
    }
}

/// Equivalent of `test_call.sh`: sends a minimal non-streaming chat
/// completion. A generous timeout is used because, in router mode, this
/// call may be what triggers the model to actually load into memory.
pub async fn chat(base_url: &str, model: &str, prompt: &str) -> Result<ChatOutcome, String> {
    let body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": prompt }
        ],
        "stream": false
    });

    let sent_at = Local::now();
    let started = std::time::Instant::now();

    let response = client()
        .post(format!("{base_url}/v1/chat/completions"))
        .timeout(CHAT_TIMEOUT)
        .json(&body)
        .send()
        .await
        .map_err(|error| unreachable(base_url, &error))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("could not read response body: {error}"))?;
    let latency = started.elapsed();

    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", text.trim()));
    }

    let value: Value =
        serde_json::from_str(&text).map_err(|error| format!("invalid JSON response: {error}"))?;

    let reply = value["choices"][0]["message"]["content"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("unexpected response shape: {text}"))?;

    Ok(ChatOutcome {
        model: model.to_string(),
        prompt: prompt.to_string(),
        reply,
        sent_at,
        latency,
        prompt_tokens: value["usage"]["prompt_tokens"].as_u64(),
        completion_tokens: value["usage"]["completion_tokens"].as_u64(),
        // llama.cpp reports this directly; fall back to deriving it from
        // the token count and our own measured latency.
        tokens_per_second: value["timings"]["predicted_per_second"]
            .as_f64()
            .or_else(|| derive_rate(value["usage"]["completion_tokens"].as_u64(), latency)),
        prompt_ms: value["timings"]["prompt_ms"].as_f64(),
        predicted_ms: value["timings"]["predicted_ms"].as_f64(),
    })
}

fn derive_rate(tokens: Option<u64>, latency: Duration) -> Option<f64> {
    let tokens = tokens? as f64;
    let seconds = latency.as_secs_f64();
    (seconds > 0.0).then(|| tokens / seconds)
}

/// Reply-only form, used by the `:ping` command.
pub async fn test_chat(base_url: &str, model: &str) -> Result<String, String> {
    chat(base_url, model, DEFAULT_PROMPT)
        .await
        .map(|outcome| outcome.reply)
}

/// Lists the model ids the server currently reports via `GET /v1/models` —
/// used by `status` to confirm the server is reachable independently of
/// the log-based "ready" heuristic in `process.rs`.
pub async fn list_models(base_url: &str) -> Result<Vec<String>, String> {
    let response = client()
        .get(format!("{base_url}/v1/models"))
        .timeout(STATUS_TIMEOUT)
        .send()
        .await
        .map_err(|error| unreachable(base_url, &error))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("could not read response body: {error}"))?;

    if !status.is_success() {
        return Err(format!("HTTP {status}: {text}"));
    }

    let value: Value =
        serde_json::from_str(&text).map_err(|error| format!("invalid JSON response: {error}"))?;

    Ok(parse_model_list(&value))
}

/// llama-server has shipped two shapes for `/v1/models`: the OpenAI one
/// (`{"data":[{"id":...}]}`) and an Ollama-flavoured one
/// (`{"models":[{"name":...,"model":...}]}` — what build 10330 returns).
/// Accept both, rather than silently reporting "no models loaded" against
/// a server that is plainly serving one.
fn parse_model_list(value: &Value) -> Vec<String> {
    let items = value["data"]
        .as_array()
        .or_else(|| value["models"].as_array());

    items
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    ["id", "name", "model"]
                        .iter()
                        .find_map(|key| item[*key].as_str())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_formats_host_and_port() {
        assert_eq!(base_url("127.0.0.1", 1234), "http://127.0.0.1:1234");
    }

    #[test]
    fn parses_the_openai_shape() {
        let value = serde_json::json!({"data": [{"id": "gemma4-12b"}, {"id": "qwen3-coder"}]});
        assert_eq!(parse_model_list(&value), vec!["gemma4-12b", "qwen3-coder"]);
    }

    /// The shape actually returned by llama-server build 10330.
    #[test]
    fn parses_the_ollama_shape() {
        let value = serde_json::json!({
            "models": [{"name": "gemma4-12b", "model": "gemma4-12b", "type": "model"}]
        });
        assert_eq!(parse_model_list(&value), vec!["gemma4-12b"]);
    }

    fn outcome(completion_tokens: Option<u64>, rate: Option<f64>) -> ChatOutcome {
        ChatOutcome {
            completion_tokens,
            tokens_per_second: rate,
            ..ChatOutcome::sample()
        }
    }

    #[test]
    fn summary_always_reports_latency() {
        assert_eq!(outcome(None, None).summary(), "gemma4-12b (1.25s)");
    }

    #[test]
    fn summary_adds_whatever_the_server_reported() {
        assert_eq!(
            outcome(Some(42), Some(33.33)).summary(),
            "gemma4-12b (1.25s, 42 tok, 33.3 tok/s)"
        );
    }

    /// A server that reports token counts but no `timings` block still
    /// gets a rate, derived from our own measured latency.
    #[test]
    fn a_missing_rate_is_derived_from_latency() {
        assert_eq!(derive_rate(Some(50), Duration::from_secs(2)), Some(25.0));
    }

    #[test]
    fn a_rate_cannot_be_derived_without_tokens_or_time() {
        assert_eq!(derive_rate(None, Duration::from_secs(2)), None);
        assert_eq!(derive_rate(Some(50), Duration::ZERO), None);
    }

    #[test]
    fn an_unknown_shape_yields_no_models_rather_than_an_error() {
        let value = serde_json::json!({"something": "else"});
        assert!(parse_model_list(&value).is_empty());
    }

    /// A port nothing is listening on. Bound and dropped, so the number is
    /// real and free rather than a guess that might collide with something
    /// the developer happens to be running.
    fn closed_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        port
    }

    /// The message this whole classification exists for.
    ///
    /// Every feature that talks to the server needs one to be serving, and
    /// when none is, reqwest reports `error sending request for url (…)` —
    /// which describes the plumbing, not the situation, and leaves the
    /// reader to work out that it means "launch something". No network is
    /// involved here: the connection is refused by the loopback interface
    /// immediately.
    #[tokio::test]
    async fn a_silent_port_is_reported_as_no_server_running() {
        let base = base_url("127.0.0.1", closed_port());

        for message in [
            list_models(&base).await.expect_err("nothing is listening"),
            chat(&base, "any-model", "hello")
                .await
                .expect_err("nothing is listening"),
        ] {
            assert!(
                message.contains("no llama-server is running"),
                "not actionable: {message}"
            );
            assert!(
                message.contains(&base),
                "the endpoint is not named: {message}"
            );
            assert!(
                message.contains(":launch") && message.contains(":router"),
                "neither way out is named: {message}"
            );
            assert!(
                !message.contains("error sending request"),
                "reqwest's plumbing message leaked through: {message}"
            );
        }
    }

    /// Live checks against a server the developer started by hand. Ignored
    /// by default; run with:
    ///   cargo test -- --ignored --test-threads=1
    mod live {
        use super::*;

        const BASE: &str = "http://127.0.0.1:1234";

        #[tokio::test]
        #[ignore = "requires a running llama-server on 127.0.0.1:1234"]
        async fn health_reports_serving() {
            assert_eq!(health(BASE).await, Health::Serving);
        }

        #[tokio::test]
        #[ignore = "requires a running llama-server on 127.0.0.1:1234"]
        async fn list_models_sees_the_loaded_model() {
            let models = list_models(BASE).await.expect("list models");
            assert!(!models.is_empty(), "server reported no models");
        }

        #[tokio::test]
        #[ignore = "requires a running llama-server on 127.0.0.1:1234"]
        async fn port_in_use_detects_the_listener() {
            assert!(port_in_use("127.0.0.1", 1234).await);
        }

        #[tokio::test]
        #[ignore = "requires nothing listening on 127.0.0.1:1"]
        async fn health_reports_unreachable_when_nothing_listens() {
            assert_eq!(health("http://127.0.0.1:1").await, Health::Unreachable);
        }
    }
}
