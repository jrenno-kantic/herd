use ratatui::widgets::Paragraph;

pub fn render_command(input: &str) -> Paragraph<'static> {
    Paragraph::new(format!(":{}", input))
}
