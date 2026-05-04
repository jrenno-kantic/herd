use ratatui::widgets::Paragraph;

pub fn render_logs(logs: &[String]) -> Paragraph<'static> {
    Paragraph::new(logs.join("\n"))
}
