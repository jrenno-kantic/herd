pub struct App {
    pub logs: Vec<String>,
    pub command_input: String,
}

impl App {
    pub fn new() -> Self {
        Self {
            logs: vec!["OPS-TUI started".into()],
            command_input: String::new(),
        }
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        if let crossterm::event::KeyCode::Char(c) = key.code {
            self.command_input.push(c);
        }
    }

    pub fn run_command(&mut self) {
        let command = self.command_input.trim();

        if command.is_empty() {
            return;
        }

        let output = crate::services::scripts::run_script(command);
        self.logs.push(format!(":{} -> {}", command, output));
        self.command_input.clear();
    }
}
