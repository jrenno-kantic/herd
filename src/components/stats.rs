//! The Stats screen: what this serving session has done, and how much
//! memory the machine is willing to give it.

use crate::{
    app::{App, Screen},
    components, keys,
    services::llama::memory,
    theme::Theme,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(5)])
        .split(area);

    frame.render_widget(session(app), chunks[0]);
    frame.render_widget(memory_budget(app, chunks[1].width), chunks[1]);
}

fn session(app: &App) -> Paragraph<'static> {
    let stats = &app.llama.stats;
    let server = &app.llama.server;

    let uptime = server
        .uptime_secs()
        .map(format_duration)
        .unwrap_or_else(|| "-".into());

    let lines = vec![
        field("model", server.model.clone().unwrap_or_else(|| "-".into())),
        field("state", server.state.label()),
        field("started", stats.started_label()),
        field("uptime", uptime),
        Line::from(""),
        field(
            "requests",
            match stats.failures {
                0 => stats.probes.to_string(),
                failures => format!("{} ({failures} failed)", stats.probes),
            },
        ),
        field("tokens in", stats.prompt_tokens.to_string()),
        field("tokens out", stats.completion_tokens.to_string()),
        field("throughput", throughput(app)),
        field("TTFT", ttft(app)),
    ];

    Paragraph::new(lines).block(block(" Session "))
}

/// Time to first token: how long the model takes to *start* answering, as
/// against how fast it answers once it has.
///
/// The two come apart exactly where it matters. A model paging its weights
/// in can generate at a perfectly respectable rate and still leave the user
/// looking at nothing for four seconds, and throughput alone reports that
/// session as healthy.
///
/// **The leading figure is the cold one** — the first probe after the load,
/// the only request that measures a model that is not yet resident. `last`
/// and `avg` follow it and describe the warm model, which is the other half
/// of the question: the first says how long until it was usable at all, and
/// these say what it costs per request once it is. They are kept apart
/// rather than averaged together, because a mean over both drifts towards
/// the warm value the more probes are run and describes neither.
///
/// Every figure here is derived from the round trip and the server's own
/// generation time rather than watched arriving — the probe is
/// non-streaming — so a server that sends no `timings` gets a plain `-`
/// and a reason rather than a zero.
fn ttft(app: &App) -> String {
    let stats = &app.llama.stats;

    let cold = match (stats.first_token, stats.probes) {
        (Some(cold), _) => seconds(cold),
        (None, 0) => {
            return format!(
                "-  (Time to First Token) · run a test on screen {}",
                screen_number(Screen::Test)
            )
        }
        // The first probe answered but its server accounts for nothing, so
        // there is no cold measurement to be had this session — and the
        // second probe is not a stand-in for it, warm as it is.
        (None, _) => "-".to_string(),
    };

    let mut parts = vec![format!("{cold}  (Time to First Token)")];
    if let Some(last) = stats.last_ttft {
        parts.push(format!("last {}", seconds(last)));
    }
    if let Some(average) = stats.average_ttft() {
        parts.push(format!("avg {}", seconds(average)));
    }

    if parts.len() == 1 {
        parts.push("that server reported no timings".to_string());
    }

    parts.join(" · ")
}

fn seconds(duration: std::time::Duration) -> String {
    format!("{:.2}s", duration.as_secs_f64())
}

/// The digit that jumps to a screen. Derived rather than written down: the
/// shortcuts are positional, so inserting a screen renumbers every one
/// after it — and this line said "screen 3" for a while after the Router
/// screen took that number.
fn screen_number(screen: Screen) -> usize {
    Screen::ALL
        .iter()
        .position(|&candidate| candidate == screen)
        .map(|index| index + 1)
        .unwrap_or_default()
}

fn throughput(app: &App) -> String {
    let stats = &app.llama.stats;

    let mut parts = Vec::new();
    if let Some(average) = stats.average_rate() {
        parts.push(format!("{average:.1} tok/s avg"));
    }
    if let Some(last) = stats.last_rate {
        parts.push(format!("{last:.1} last"));
    }
    if let Some(best) = stats.best_rate {
        parts.push(format!("{best:.1} best"));
    }

    if parts.is_empty() {
        format!("-  (run a test on screen {})", screen_number(Screen::Test))
    } else {
        parts.join("  ·  ")
    }
}

fn memory_budget(app: &App, width: u16) -> Paragraph<'static> {
    let budget = app.llama.budget();
    let risky = budget.is_risky();

    let total = match app.llama.ram_gib {
        Some(gib) => format!("{gib} GiB"),
        None => "unknown".to_string(),
    };

    let mut lines = vec![
        field("installed", total),
        Line::from(vec![
            Span::styled("  reserved  ", Theme::logs()),
            Span::styled(
                format!(
                    "{:.0}%  ({:.1} GiB)",
                    budget.reserved_ratio * 100.0,
                    budget.reserved_gib()
                ),
                if risky {
                    Theme::status_error()
                } else {
                    Theme::normal()
                },
            ),
        ]),
        field("for models", format!("{:.1} GiB", budget.available_gib())),
        Line::from(""),
    ];

    if risky {
        lines.push(Line::styled(
            "  ⚠ CAUTION  less is reserved than the system default.",
            Theme::status_error(),
        ));
        lines.push(Line::styled(
            "    The OS may swap, stall, or kill the server under load.",
            Theme::status_error(),
        ));
        lines.push(Line::from(""));
    }

    lines.push(Line::styled(
        format!(
            "  This only changes how herd judges whether a preset fits (default {:.0}%).",
            memory::DEFAULT_RESERVED_RATIO * 100.0
        ),
        Theme::logs(),
    ));
    lines.push(Line::styled(
        "  It does not change any system setting. On macOS the real GPU limit is",
        Theme::logs(),
    ));
    lines.push(Line::styled(
        "  sudo sysctl iogpu.wired_limit_mb=<MB> — run that yourself if you mean it.",
        Theme::logs(),
    ));
    lines.push(Line::from(""));
    lines.push(Line::styled(
        format!(
            "  {}",
            keys::screen_hint_within(Screen::Stats, components::hint_width(width, true, 0))
        ),
        Theme::logs(),
    ));

    Paragraph::new(lines).block(block(" Memory budget "))
}

fn field(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<12}"), Theme::logs()),
        Span::styled(value, Theme::normal()),
    ])
}

fn format_duration(total: u64) -> String {
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

fn block(title: &'static str) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Theme::border())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::services::llama::api::ChatOutcome;
    use crate::{event::UiEvent, services::llama::memory::DEFAULT_RESERVED_RATIO};
    use std::path::PathBuf;
    use std::time::Duration;

    fn app() -> App {
        App::with_config_path(PathBuf::from("/nonexistent/models.ini"))
    }

    fn probe(app: &mut App, completion: u64, rate: f64, seconds: u64) {
        app.update(UiEvent::ChatResult(Box::new(Ok(ChatOutcome {
            latency: Duration::from_secs(seconds),
            prompt_tokens: Some(10),
            completion_tokens: Some(completion),
            tokens_per_second: Some(rate),
            ..ChatOutcome::sample()
        }))));
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(format_duration(9), "9s");
        assert_eq!(format_duration(70), "1m10s");
        assert_eq!(format_duration(3725), "1h02m05s");
    }

    /// The pointer is derived from `Screen::ALL`, so inserting a screen
    /// cannot leave it naming the wrong one — which is what happened when
    /// the Router screen took number 3.
    #[test]
    fn throughput_points_at_the_test_screen_before_any_probe() {
        let expected = format!("screen {}", screen_number(Screen::Test));

        assert!(throughput(&app()).contains(&expected));
        assert!(ttft(&app()).contains(&expected));
    }

    /// Sends a probe whose server accounts for `predicted` ms of
    /// generation, so a TTFT can be derived from it.
    fn timed_probe(app: &mut App, latency: u64, predicted: f64) {
        app.update(UiEvent::ChatResult(Box::new(Ok(ChatOutcome {
            latency: Duration::from_secs(latency),
            predicted_ms: Some(predicted),
            ..ChatOutcome::sample()
        }))));
    }

    /// TTFT and throughput answer different questions, and the case that
    /// separates them is a model that is slow to *start*: 5s of wait, then
    /// 100 tokens in 1s. Throughput calls that fast; the wait is what the
    /// user sat through.
    #[test]
    fn time_to_first_token_is_the_wait_before_generation() {
        let mut app = app();
        timed_probe(&mut app, 6, 1_000.0);

        let text = ttft(&app);
        assert!(text.contains("5.00s"), "{text}");
        assert!(text.contains("Time to First Token"), "{text}");
    }

    /// **The leading figure is the cold one**, and stays cold. Every probe
    /// after the first is answered from resident weights and a warm cache,
    /// so a later — invariably faster — one must not replace it or be
    /// averaged into it; those go to `last` and `avg` beside it, which
    /// describe the warm model on purpose.
    #[test]
    fn the_leading_figure_stays_the_cold_one() {
        let mut app = app();
        timed_probe(&mut app, 6, 1_000.0); // cold: 5s
        timed_probe(&mut app, 2, 1_000.0); // warm: 1s
        timed_probe(&mut app, 1, 500.0); // warm: 0.5s

        assert_eq!(app.llama.stats.first_token, Some(Duration::from_secs(5)));

        let text = ttft(&app);
        assert!(text.starts_with("5.00s"), "the cold figure moved: {text}");
        assert!(text.contains("last 0.50s"), "{text}");
        // (5 + 1 + 0.5) / 3 = 2.166…, and the average covers the cold
        // probe too: it is the mean over what was measured, where the
        // leading figure is one specific probe.
        assert!(text.contains("avg 2.17s"), "{text}");
    }

    /// One probe is cold, last and average all at once, and saying so
    /// three times is correct rather than a bug — the numbers only diverge
    /// once there is a second probe to diverge from.
    #[test]
    fn a_single_probe_is_its_own_last_and_average() {
        let mut app = app();
        timed_probe(&mut app, 6, 1_000.0);

        let text = ttft(&app);
        assert!(
            text.contains("last 5.00s") && text.contains("avg 5.00s"),
            "{text}"
        );
    }

    /// ...and a new launch starts the measurement again, since the model
    /// it describes is a different one — or the same one, cold again.
    #[test]
    fn a_new_launch_measures_the_next_first_call() {
        let mut app = app();
        timed_probe(&mut app, 6, 1_000.0);

        app.update(UiEvent::LlamaStatus(
            crate::services::llama::LlamaSnapshot::new(
                crate::services::llama::ServerState::Starting,
                crate::services::llama::LauncherMode::Manual,
                Some("gemma4-12b".into()),
            ),
        ));
        assert_eq!(app.llama.stats.first_token, None, "the old figure survived");

        timed_probe(&mut app, 9, 1_000.0);
        assert_eq!(app.llama.stats.first_token, Some(Duration::from_secs(8)));
    }

    /// A server that sends no `timings` has no TTFT to report, and must
    /// say so rather than showing a zero — the same restraint as
    /// `Fit::Unknown`.
    #[test]
    fn a_server_without_timings_reports_nothing_rather_than_zero() {
        let mut app = app();
        probe(&mut app, 10, 5.0, 1);

        assert_eq!(app.llama.stats.first_token, None);
        assert!(ttft(&app).contains("no timings"), "{}", ttft(&app));
        assert!(!ttft(&app).contains("0.00s"), "{}", ttft(&app));
    }

    /// The first probe reporting nothing does not promote the second: by
    /// then the model is warm, so there is no cold measurement to be had
    /// this session. The dash says so, and the warm figures still show —
    /// they are true, and they are what there is.
    #[test]
    fn a_warm_probe_is_never_promoted_to_the_cold_figure() {
        let mut app = app();
        probe(&mut app, 10, 5.0, 1); // no timings: no cold measurement
        timed_probe(&mut app, 6, 1_000.0); // warm, and timed

        assert_eq!(app.llama.stats.first_token, None);

        let text = ttft(&app);
        assert!(
            text.starts_with('-'),
            "a warm probe took the cold slot: {text}"
        );
        assert!(text.contains("last 5.00s"), "{text}");
    }

    /// The average must come from totals, not from averaging the
    /// per-request rates: a slow first request would otherwise be weighted
    /// the same as a fast later one.
    #[test]
    fn throughput_averages_over_the_whole_session() {
        let mut app = app();
        probe(&mut app, 100, 50.0, 4); // 100 tokens in 4s
        probe(&mut app, 100, 25.0, 1); // 100 tokens in 1s

        // 200 tokens over 5s = 40 tok/s, not (50+25)/2.
        let text = throughput(&app);
        assert!(text.contains("40.0 tok/s avg"), "{text}");
        assert!(text.contains("25.0 last"), "{text}");
        assert!(text.contains("50.0 best"), "{text}");
    }

    #[test]
    fn failures_are_counted_separately() {
        let mut app = app();
        probe(&mut app, 10, 5.0, 1);
        app.update(UiEvent::ChatResult(Box::new(Err("boom".into()))));

        assert_eq!(app.llama.stats.probes, 1);
        assert_eq!(app.llama.stats.failures, 1);
    }

    #[test]
    fn the_default_budget_is_not_flagged_risky() {
        let app = app();
        assert_eq!(app.llama.reserved_ratio, DEFAULT_RESERVED_RATIO);
        assert!(!app.llama.budget().is_risky());
    }
}
