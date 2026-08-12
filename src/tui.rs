use crate::{
    app::{App, Mode, Screen},
    components::{
        command_bar, confirm, help, hub, logs, models, picker, router, server, settings, sidebar,
        stats, status, test,
    },
    layout,
    theme::Theme,
};
use anyhow::Result;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, widgets::Block, Frame, Terminal};
use std::io;

type Backend = CrosstermBackend<io::Stdout>;

pub struct TerminalSession {
    terminal: Terminal<Backend>,
}

impl TerminalSession {
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;

        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        Ok(Self { terminal })
    }

    pub fn draw(&mut self, app: &App) -> Result<()> {
        self.terminal.draw(|frame| render(frame, app))?;
        Ok(())
    }
}

/// The whole UI as a pure function of `App`. Extracted from `draw` so it
/// can be exercised against a `TestBackend` without a real terminal.
pub fn render(frame: &mut Frame, app: &App) {
    let areas = layout::main(frame.area());

    frame.render_widget(Block::default().style(Theme::background()), frame.area());
    frame.render_widget(sidebar::view(app), areas.sidebar);

    match app.screen {
        Screen::Models => models::render(frame, app, areas.main),
        Screen::Hub => hub::render(frame, app, areas.main),
        Screen::Server => server::render(frame, app, areas.main),
        Screen::Router => router::render(frame, app, areas.main),
        Screen::Test => test::render(frame, app, areas.main),
        Screen::Stats => stats::render(frame, app, areas.main),
        Screen::Settings => settings::render(frame, app, areas.main),
        Screen::Logs => logs::render(frame, app, areas.main),
    }

    frame.render_widget(command_bar::view(app), areas.command);
    frame.render_widget(status::view(app), areas.status);

    if app.mode == Mode::ConfirmLaunch {
        confirm::render(frame, app, frame.area());
    }
    if app.mode == Mode::ConfirmQuit {
        confirm::render_quit(frame, app, frame.area());
    }
    if app.mode == Mode::Picker {
        picker::render(frame, app, frame.area());
    }
    if app.mode == Mode::Help {
        help::render(frame, app, frame.area());
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use std::io::Write;

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
"#;

    fn sample_app() -> App {
        // The thread id is part of the name, not just the pid: tests run in
        // parallel, and two of them creating the same path meant one
        // truncating the file the other was about to read — which surfaces
        // as a screen with no presets on it and a failure that moves
        // around between runs.
        let path = std::env::temp_dir().join(format!(
            "herd-render-{}-{:?}.ini",
            std::process::id(),
            std::thread::current().id()
        ));
        let mut file = std::fs::File::create(&path).expect("create sample ini");
        file.write_all(SAMPLE_INI.as_bytes()).expect("write ini");
        App::with_config_path(path)
    }

    fn shipped(tier: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join(tier)
            .join("models.ini")
    }

    fn frame_text(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal.draw(|frame| render(frame, app)).expect("draw");

        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_screen_renders_without_panicking() {
        let mut app = sample_app();
        for screen in Screen::ALL {
            app.screen = screen;
            let text = frame_text(&app, 120, 40);
            assert!(text.contains(screen.label()), "{screen:?} title missing");
        }
    }

    #[test]
    fn the_models_screen_shows_the_preset_and_its_argv() {
        let app = sample_app();
        let text = frame_text(&app, 120, 40);

        assert!(text.contains("gemma4-12b"), "preset row missing");
        assert!(text.contains("unsloth/gemma-4-12B"), "repo column missing");
        assert!(text.contains("32768"), "inherited ctx-size missing");
        assert!(text.contains("llama-server"), "argv preview missing");
    }

    /// The copy key does not fit in the Models footer, so the preview
    /// border is the only place it is advertised. A feature nobody can
    /// find is not a feature — and this is also what pins the hint to the
    /// pane it acts on, at the narrow width where the footer gave up.
    #[test]
    fn the_argv_preview_says_how_to_copy_it() {
        let app = sample_app();

        for width in [100, 120] {
            let text = frame_text(&app, width, 40);
            assert!(
                text.contains("y copy"),
                "no copy hint on the argv preview at {width} columns"
            );
        }
    }

    #[test]
    fn a_starred_preset_is_drawn_with_its_star() {
        let mut app = sample_app();
        assert!(!frame_text(&app, 120, 40).contains('★'), "star before any");

        app.update(crate::event::UiEvent::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('f'),
            crossterm::event::KeyModifiers::NONE,
        )));

        let text = frame_text(&app, 120, 40);
        assert!(text.contains('★'), "no star on the starred row:\n{text}");
    }

    /// The Router screen has to say what it would start and what it would
    /// start it with — the two numbers were flags nobody could see when
    /// `:router` was the only way in.
    #[test]
    fn the_router_screen_shows_its_settings_and_the_argv() {
        let mut app = sample_app();
        app.screen = Screen::Router;

        let text = frame_text(&app, 120, 40);
        assert!(text.contains("models-max"), "{text}");
        assert!(text.contains("sleep-idle-seconds"), "{text}");
        assert!(text.contains("--models-preset"), "no argv preview:\n{text}");
        assert!(text.contains("not running"), "no state line:\n{text}");
    }

    /// Prints the Models and Router screens at two widths, because
    /// arithmetic saying it fits is not the same as looking at it.
    #[test]
    #[ignore = "prints the new screens for inspection"]
    fn show_the_new_screens() {
        let mut app = App::with_config_path(shipped("32gb"));
        app.llama.favorites.insert("gemma4-12b".into());
        app.update(crate::event::UiEvent::Resize { height: 32 });

        // The real cache of the machine this is run on, sizes and all —
        // the Hub screen is not worth looking at against a fixture.
        if let Ok(cached) = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(crate::services::llama::hub::cache_list())
        {
            app.update(crate::event::UiEvent::CacheList(cached));
        }

        for width in [100, 120] {
            for screen in [Screen::Models, Screen::Hub, Screen::Router] {
                app.screen = screen;
                println!(
                    "\n=== {screen:?} at {width} ===\n{}",
                    frame_text(&app, width, 32)
                );
            }
        }
    }

    /// The one assertion that keeps `app::chrome` honest.
    ///
    /// A page is derived from the terminal height minus a per-screen
    /// allowance for everything that is not a list row, and those
    /// allowances are hand-counted from the layout — so they can drift the
    /// moment a screen grows a line. Paging further than the screen shows
    /// skips content silently, which is the failure worth catching: draw
    /// the real thing, count the rows that actually appeared, and require
    /// the page to fit inside them.
    #[test]
    fn a_page_never_moves_further_than_the_rows_on_screen() {
        // Deliberately far more entries than any test height can show, on
        // every screen, so the rows counted below are the ones the
        // *viewport* allowed and not simply the end of a short list.
        let mut ini = String::from("[server]\nhost = 127.0.0.1\nport = 1234\n\n[*]\n");
        for i in 0..200 {
            ini.push_str(&format!("opt-{i:03} = {i}\n"));
        }
        ini.push('\n');
        for i in 0..200 {
            ini.push_str(&format!(
                "[preset-{i:03}]\nhf-repo = vendor/model-{i:03}\n\n"
            ));
        }
        let path = std::env::temp_dir().join(format!("herd-page-{}.ini", std::process::id()));
        std::fs::write(&path, ini).expect("write ini");

        let mut app = App::with_config_path(path);
        for i in 0..500 {
            app.push_log(format!("logline-{i:03}"));
        }

        // One needle per screen, appearing on its list rows and nowhere
        // else, so counting matches counts visible rows.
        let needles = [
            (Screen::Models, "preset-"),
            (Screen::Settings, "opt-"),
            (Screen::Logs, "logline-"),
        ];

        for (screen, needle) in needles {
            app.screen = screen;

            for height in [24, 40, 60] {
                app.update(crate::event::UiEvent::Resize { height });

                let drawn = frame_text(&app, 120, height)
                    .lines()
                    .filter(|line| line.contains(needle))
                    .count();

                assert!(
                    app.page() <= drawn,
                    "{screen:?} at {height} rows pages by {} but draws only {drawn}",
                    app.page()
                );
            }
        }
    }

    /// A scrollbar on the right border, and only when there is something
    /// to scroll: a full-height thumb over a four-line buffer would imply
    /// a scrollback that does not exist.
    #[test]
    fn the_logs_screen_draws_a_scrollbar_only_when_it_can_scroll() {
        let mut app = sample_app();
        app.screen = Screen::Logs;

        let bar = |text: &str| text.chars().filter(|c| "█░▐▌".contains(*c)).count();

        let short = frame_text(&app, 120, 40);
        assert_eq!(bar(&short), 0, "a buffer that fits needs no scrollbar");

        for i in 0..500 {
            app.push_log(format!("line {i}"));
        }
        let long = frame_text(&app, 120, 40);
        assert!(bar(&long) > 0, "no scrollbar over a 500-line buffer");
    }

    /// Glyphs a ratatui scrollbar draws, track and thumb.
    fn scrollbar_cells(text: &str) -> usize {
        text.chars().filter(|c| "█░▐▌".contains(*c)).count()
    }

    /// The same rule the Logs screen follows: a bar only where there is
    /// something to scroll. A full-height thumb beside a list that fits
    /// would imply presets below the fold that do not exist.
    #[test]
    fn the_models_table_draws_a_scrollbar_only_when_it_overflows() {
        let short = sample_app();
        assert_eq!(
            scrollbar_cells(&frame_text(&short, 120, 40)),
            0,
            "a single preset does not need a scrollbar"
        );

        let tall = App::with_config_path(shipped("16gb"));
        assert!(tall.llama.rows().len() > 6, "fixture needs more presets");
        assert!(
            scrollbar_cells(&frame_text(&tall, 120, 14)) > 0,
            "no scrollbar over a list taller than the screen"
        );
        assert_eq!(
            scrollbar_cells(&frame_text(&tall, 120, 40)),
            0,
            "the same list drawn in full still got a scrollbar"
        );
    }

    /// The Hub screen: what is in the cache, what it costs, and which of
    /// it this tier cannot launch.
    #[test]
    fn the_hub_screen_lists_the_cache_and_flags_what_no_preset_names() {
        let mut app = sample_app();
        app.screen = Screen::Hub;

        // Before llama.cpp answers, an empty list is not a claim.
        assert!(frame_text(&app, 120, 40).contains("asking llama-server"));

        app.update(crate::event::UiEvent::CacheList(vec![
            crate::services::llama::hub::CachedModel {
                weights: Some(7_193_575_424),
                bytes: Some(22_106_570_752),
                ..crate::services::llama::hub::CachedModel::from(
                    "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL",
                )
            },
            crate::services::llama::hub::CachedModel {
                weights: Some(3_800_000_000),
                bytes: Some(3_800_000_000),
                ..crate::services::llama::hub::CachedModel::from("vendor/Orphan-4B-GGUF:Q4_K_M")
            },
        ]));

        let text = frame_text(&app, 120, 40);
        assert!(text.contains("gemma-4-12B-it-qat-GGUF:Q4_K_XL"), "{text}");
        assert!(text.contains("6.7G"), "no model size: {text}");
        assert!(text.contains("20.6G"), "no disk usage: {text}");
        assert!(
            text.contains("gemma4-12b"),
            "the preset using it is not named: {text}"
        );
        assert!(
            text.contains("1 not named by this tier"),
            "the unreferenced model is not counted: {text}"
        );
        assert!(text.contains("y copy preset"), "no copy hint: {text}");
    }

    /// The measured size and the estimate are not shown as equally
    /// certain: the estimate carries a `~`, and once the weights are on
    /// disk it goes away.
    #[test]
    fn the_models_table_marks_an_estimated_size() {
        let mut app = sample_app();
        assert!(
            frame_text(&app, 120, 40).contains('~'),
            "an estimated size is not marked as one"
        );

        app.update(crate::event::UiEvent::CacheList(vec![
            crate::services::llama::hub::CachedModel {
                weights: Some(7_193_575_424),
                bytes: Some(7_193_575_424),
                ..crate::services::llama::hub::CachedModel::from(
                    "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL",
                )
            },
        ]));

        let text = frame_text(&app, 120, 40);
        assert!(
            text.contains("7.7G"),
            "the measured size is missing: {text}"
        );
    }

    /// "not local" is the difference between pressing Enter and waiting a
    /// second, and pressing Enter and waiting twenty minutes — so it has to
    /// be visible in the list, not only in a prompt after the fact.
    #[test]
    fn the_models_screen_shows_what_is_not_downloaded() {
        let mut app = sample_app();

        // Before the cache has been read, nothing is claimed either way.
        assert!(!frame_text(&app, 120, 40).contains("not local"));

        app.update(crate::event::UiEvent::CacheList(vec![]));
        assert!(
            frame_text(&app, 120, 40).contains("not local"),
            "a missing preset is not called out"
        );

        app.update(crate::event::UiEvent::CacheList(vec![
            "unsloth/gemma-4-12B-it-qat-GGUF:Q4_K_XL".into(),
        ]));
        assert!(
            !frame_text(&app, 120, 40).contains("not local"),
            "a cached preset is wrongly called missing"
        );
    }

    /// The bar replaces the argv preview while a download runs, and states
    /// bytes rather than a bare percentage.
    #[test]
    fn a_download_shows_a_progress_bar() {
        let mut app = sample_app();
        app.update(crate::event::UiEvent::DownloadProgress {
            model: "gemma4-12b".into(),
            done: 2_147_483_648,
            total: 6_871_947_674,
        });

        let text = frame_text(&app, 120, 40);
        assert!(text.contains("downloading gemma4-12b"), "{text}");
        assert!(text.contains("2.0G"), "no byte counter: {text}");
        assert!(text.contains("31%"), "no percentage: {text}");
        assert!(
            !text.contains("argv preview"),
            "the bar did not take the pane"
        );
    }

    /// The Models footer used to put the preset description and the key
    /// hints on one line, and the description — which now spells out
    /// optimisations and capabilities — pushed the hints off the right
    /// edge. Both must be fully visible.
    #[test]
    fn the_models_footer_shows_the_description_and_every_key() {
        let app = sample_app();
        let text = frame_text(&app, 120, 40);

        assert!(text.contains("RAM"), "no description line: {text}");
        // The hint that used to be truncated away.
        assert!(text.contains("t/T tier"), "the key hints are still cut off");
        assert!(
            text.contains("d download"),
            "the key hints are still cut off"
        );
    }

    /// The description line must name the mechanism: "speculative
    /// decoding" alone does not say it is MTP, which is the part worth
    /// reading when a model ships more than one head.
    #[test]
    fn the_description_line_names_the_speculative_head() {
        let mut app = App::with_config_path(shipped("16gb"));

        let mtp = app
            .llama
            .rows()
            .iter()
            .position(|row| row.spec == "mtp")
            .expect("the 16gb tier has an MTP preset");
        app.llama.cursor = mtp;

        let text = frame_text(&app, 120, 40);
        assert!(
            text.contains("speculative decoding (mtp)"),
            "the head is not named: {text}"
        );
    }

    /// Scrolling a list whose length and position are invisible is
    /// guesswork: a cursor stopped at the end looks exactly like one that
    /// stopped responding.
    #[test]
    fn a_list_shows_where_the_cursor_is() {
        let mut app = sample_app();
        assert!(
            frame_text(&app, 120, 40).contains("1/1"),
            "the Models list has no position indicator"
        );

        app.screen = Screen::Settings;
        let text = frame_text(&app, 120, 40);
        assert!(text.contains('/'), "the Settings list has no position");
    }

    /// Quitting mid-download must say what would be abandoned, not just
    /// ask "are you sure".
    #[test]
    fn the_quit_prompt_names_the_work_in_flight() {
        let mut app = sample_app();
        app.update(crate::event::UiEvent::DownloadProgress {
            model: "gemma4-12b".into(),
            done: 1_073_741_824,
            total: 4_294_967_296,
        });
        app.update(crate::event::UiEvent::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('q'),
            crossterm::event::KeyModifiers::NONE,
        )));

        let text = frame_text(&app, 120, 40);
        assert!(text.contains("Work in progress"), "{text}");
        assert!(text.contains("downloading gemma4-12b"), "{text}");
        assert!(
            text.contains("stopped on exit"),
            "the server note is missing"
        );
    }

    /// The Server screen, serving the preset the cursor is on — the case
    /// where Enter is refused.
    #[test]
    #[ignore = "prints the Server screen for inspection"]
    fn show_the_server_screen() {
        let mut app = App::with_config_path(shipped("16gb"));
        app.screen = Screen::Server;
        app.update(crate::event::UiEvent::LlamaStatus(
            crate::services::llama::LlamaSnapshot::new(
                crate::services::llama::ServerState::Serving,
                crate::services::llama::LauncherMode::Manual,
                app.llama.selected_model(),
            ),
        ));

        for line in frame_text(&app, 120, 30).lines().take(14) {
            eprintln!("{line}");
        }
    }

    /// A look at the real table at two widths, so the column layout is
    /// judged by what it draws rather than by its own arithmetic.
    #[test]
    #[ignore = "prints the Models table for inspection"]
    fn show_the_models_table() {
        // The 16gb tier against this machine's real cache: most of it is
        // not downloaded, which is the mixture worth looking at — measured
        // sizes, `~` estimates and "not local" on the same screen.
        let mut app = App::with_config_path(shipped("16gb"));
        if let Ok(cached) = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(crate::services::llama::hub::cache_list())
        {
            app.update(crate::event::UiEvent::CacheList(cached));
        }

        for width in [120u16, 100] {
            eprintln!("\n===== {width} columns =====");
            for line in frame_text(&app, width, 20).lines().take(20) {
                eprintln!("{line}");
            }
        }
    }

    #[test]
    fn the_status_bar_shows_the_lifecycle_state() {
        let app = sample_app();
        assert!(frame_text(&app, 120, 40).contains("OFF"));
    }

    #[test]
    fn the_port_conflict_modal_is_drawn_over_the_screen() {
        let mut app = sample_app();
        app.mode = Mode::ConfirmLaunch;
        app.llama.confirm = Some(crate::app::Confirm::PortInUse(1234));
        app.llama.pending_launch = Some("gemma4-12b".into());

        let text = frame_text(&app, 120, 40);
        assert!(text.contains("Port in use"));
        assert!(text.contains("1234"));
    }

    /// The same modal, other reason: an oversized preset must name both
    /// numbers, not just refuse.
    #[test]
    fn the_memory_warning_modal_is_drawn_over_the_screen() {
        let mut app = sample_app();
        app.mode = Mode::ConfirmLaunch;
        app.llama.confirm = Some(crate::app::Confirm::TooLarge {
            estimate: 18.2,
            budget: 12.0,
        });
        app.llama.pending_launch = Some("gemma4-12b".into());

        let text = frame_text(&app, 120, 40);
        assert!(text.contains("Not enough memory"), "{text}");
        assert!(text.contains("18.2"), "{text}");
    }

    /// A server that has gone quiet must say so on screen: the whole point
    /// of the continuous health poll is that SERVING stops being a claim
    /// nothing ever re-checks.
    #[test]
    fn a_stalled_server_says_so_in_the_status_bar() {
        let mut app = sample_app();
        app.update(crate::event::UiEvent::LlamaStatus(
            crate::services::llama::LlamaSnapshot::new(
                crate::services::llama::ServerState::Serving,
                crate::services::llama::LauncherMode::Manual,
                Some("gemma4-12b".into()),
            )
            .with_phase(crate::services::llama::Phase::Unresponsive(3)),
        ));

        assert!(
            frame_text(&app, 120, 40).contains("not responding"),
            "a stalled server still reads as healthy"
        );
    }

    /// The bug the `List` conversion fixes: the table used to render every
    /// row and let ratatui clip the overflow, so on a short terminal `j`
    /// moved a selection that had scrolled off the bottom of the screen.
    #[test]
    fn the_selected_preset_stays_visible_in_a_short_terminal() {
        let mut app = App::with_config_path(shipped("16gb"));
        let rows = app.llama.rows();
        assert!(rows.len() > 6, "fixture needs more presets than fit");

        let last = rows.len() - 1;
        app.llama.cursor = last;

        let text = frame_text(&app, 120, 20);
        assert!(
            text.contains(rows[last].name.as_str()),
            "the selected row scrolled out of view"
        );
        assert!(
            !text.contains(rows[0].name.as_str()),
            "the viewport still starts at the first row"
        );
    }

    #[test]
    fn the_help_overlay_is_drawn_over_the_screen() {
        let mut app = sample_app();
        app.mode = Mode::Help;

        let text = frame_text(&app, 120, 40);
        assert!(text.contains("Keys"));
        assert!(text.contains("launch the highlighted preset"));
        assert!(text.contains("quit"), "globals missing from the overlay");
    }

    /// A terminal too small for the layout must degrade rather than panic:
    /// every screen, and every modal drawn over it, is asked for rects the
    /// screen cannot give.
    #[test]
    fn a_tiny_terminal_does_not_panic() {
        let mut app = sample_app();

        for screen in Screen::ALL {
            app.screen = screen;
            let _ = frame_text(&app, 20, 6);
        }

        app.llama.confirm = Some(crate::app::Confirm::PortInUse(1234));
        for mode in [Mode::Help, Mode::Picker, Mode::ConfirmLaunch] {
            app.mode = mode;
            let _ = frame_text(&app, 20, 6);
        }
    }
}
