use crate::event::UiEvent;
use crossterm::event::{KeyCode, KeyEvent};
use std::collections::VecDeque;

const MAX_LOGS: usize = 500;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Focus {
    Sidebar,
    Command,
}

#[derive(Debug, Clone)]
pub enum Action {
    None,
    Quit,
    RunCommand(String),
}

#[derive(Debug)]
pub struct App {
    pub command_input: String,
    pub focus: Focus,
    pub logs: VecDeque<String>,
    pub running: bool,
    pub selected_sidebar: usize,
}

impl App {
    pub fn new() -> Self {
        let mut logs = VecDeque::with_capacity(MAX_LOGS);
        logs.push_back("OPS-TUI started".into());

        Self {
            command_input: String::new(),
            focus: Focus::Command,
            logs,
            running: false,
            selected_sidebar: 0,
        }
    }

    pub fn push_log(&mut self, entry: impl AsRef<str>) {
        for line in entry.as_ref().lines() {
            if self.logs.len() >= MAX_LOGS {
                self.logs.pop_front();
            }
            self.logs.push_back(line.to_string());
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
            UiEvent::Quit => Action::Quit,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('q') if self.command_input.is_empty() => Action::Quit,
            KeyCode::Esc => {
                self.command_input.clear();
                self.focus = Focus::Sidebar;
                Action::None
            }
            KeyCode::Char(':') => {
                self.focus = Focus::Command;
                Action::None
            }
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Sidebar => Focus::Command,
                    Focus::Command => Focus::Sidebar,
                };
                Action::None
            }
            KeyCode::Down => {
                self.selected_sidebar = (self.selected_sidebar + 1).min(2);
                Action::None
            }
            KeyCode::Up => {
                self.selected_sidebar = self.selected_sidebar.saturating_sub(1);
                Action::None
            }
            KeyCode::Backspace if self.focus == Focus::Command => {
                self.command_input.pop();
                Action::None
            }
            KeyCode::Enter if self.focus == Focus::Command => self.submit_command(),
            KeyCode::Char(c) if self.focus == Focus::Command => {
                self.command_input.push(c);
                Action::None
            }
            _ => Action::None,
        }
    }

    fn submit_command(&mut self) -> Action {
        let command = self.command_input.trim().to_string();

        if command.is_empty() || self.running {
            return Action::None;
        }

        self.running = true;
        self.push_log(format!("queued :{}", command));
        self.command_input.clear();

        Action::RunCommand(command)
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
    use crossterm::event::{KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> UiEvent {
        UiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn q_quits_when_input_is_empty() {
        let mut app = App::new();
        assert!(matches!(app.update(key(KeyCode::Char('q'))), Action::Quit));
    }

    #[test]
    fn q_is_typed_when_input_has_content() {
        let mut app = App::new();
        app.command_input.push_str("hello");
        let action = app.update(key(KeyCode::Char('q')));
        assert!(matches!(action, Action::None));
        assert_eq!(app.command_input, "helloq");
    }

    #[test]
    fn tab_toggles_focus() {
        let mut app = App::new();
        assert_eq!(app.focus, Focus::Command);
        app.update(key(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Sidebar);
        app.update(key(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Command);
    }

    #[test]
    fn colon_focuses_command() {
        let mut app = App::new();
        app.focus = Focus::Sidebar;
        app.update(key(KeyCode::Char(':')));
        assert_eq!(app.focus, Focus::Command);
    }

    #[test]
    fn esc_clears_input_and_focuses_sidebar() {
        let mut app = App::new();
        app.command_input.push_str("scan");
        app.update(key(KeyCode::Esc));
        assert!(app.command_input.is_empty());
        assert_eq!(app.focus, Focus::Sidebar);
    }

    #[test]
    fn typed_chars_append_to_command_input() {
        let mut app = App::new();
        app.update(key(KeyCode::Char('s')));
        app.update(key(KeyCode::Char('h')));
        assert_eq!(app.command_input, "sh");
    }

    #[test]
    fn backspace_pops_command_input() {
        let mut app = App::new();
        app.command_input.push_str("test");
        app.update(key(KeyCode::Backspace));
        assert_eq!(app.command_input, "tes");
    }

    #[test]
    fn enter_submits_non_empty_command() {
        let mut app = App::new();
        app.command_input.push_str("test");
        let action = app.update(key(KeyCode::Enter));
        match action {
            Action::RunCommand(cmd) => assert_eq!(cmd, "test"),
            other => panic!("expected RunCommand, got {:?}", other),
        }
        assert!(app.running);
        assert!(app.command_input.is_empty());
    }

    #[test]
    fn enter_with_empty_input_is_noop() {
        let mut app = App::new();
        let action = app.update(key(KeyCode::Enter));
        assert!(matches!(action, Action::None));
        assert!(!app.running);
    }

    #[test]
    fn enter_while_running_is_noop_and_preserves_input() {
        let mut app = App::new();
        app.running = true;
        app.command_input.push_str("test");
        let action = app.update(key(KeyCode::Enter));
        assert!(matches!(action, Action::None));
        assert_eq!(app.command_input, "test");
    }

    #[test]
    fn down_clamps_at_upper_bound() {
        let mut app = App::new();
        for _ in 0..10 {
            app.update(key(KeyCode::Down));
        }
        assert_eq!(app.selected_sidebar, 2);
    }

    #[test]
    fn up_clamps_at_zero() {
        let mut app = App::new();
        app.selected_sidebar = 2;
        for _ in 0..10 {
            app.update(key(KeyCode::Up));
        }
        assert_eq!(app.selected_sidebar, 0);
    }

    #[test]
    fn command_finished_clears_running_and_logs_result() {
        let mut app = App::new();
        app.running = true;
        let initial = app.logs.len();
        app.update(UiEvent::CommandFinished {
            command: "test".into(),
            output: "ok".into(),
        });
        assert!(!app.running);
        assert_eq!(app.logs.len(), initial + 1);
        let last = app.logs.back().unwrap();
        assert!(last.contains("test") && last.contains("ok"));
    }

    #[test]
    fn tick_is_noop() {
        let mut app = App::new();
        assert!(matches!(app.update(UiEvent::Tick), Action::None));
    }

    #[test]
    fn quit_event_returns_quit_action() {
        let mut app = App::new();
        assert!(matches!(app.update(UiEvent::Quit), Action::Quit));
    }

    #[test]
    fn push_log_splits_multiline_entries() {
        let mut app = App::new();
        let initial = app.logs.len();
        app.push_log("line one\nline two\nline three");
        assert_eq!(app.logs.len(), initial + 3);
    }

    #[test]
    fn push_log_rotates_at_capacity() {
        let mut app = App::new();
        for i in 0..MAX_LOGS + 50 {
            app.push_log(format!("entry {}", i));
        }
        assert_eq!(app.logs.len(), MAX_LOGS);
        let newest = app.logs.back().unwrap();
        assert!(newest.contains(&(MAX_LOGS + 49).to_string()));
    }
}
