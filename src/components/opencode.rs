//! The `o` overlay on the Models screen: the `opencode.json` provider
//! block for the highlighted preset, and where it goes.
//!
//! The Models screen already answers "what would this launch?" with the
//! argv preview. This answers the next question a coding session asks —
//! "and how do I point my editor at it?" — from the same facts, so the
//! port, the alias and the context size cannot be transcribed wrong on
//! the way. See `services::llama::opencode` for what the block claims and
//! what it deliberately leaves out.
//!
//! Read-only, like `?`, `:help` and `:about`. herd does not write
//! `~/.config/opencode/opencode.json`: it is hand-maintained, it belongs
//! to another program, and rewriting it would lose its comments and key
//! order — the same rule that keeps `models.ini` untouched.

use crate::{
    app::App,
    components::{self, centered},
    services::llama::opencode,
    theme::Theme,
};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// Two spaces, as every other overlay indents by.
const INDENT: usize = 2;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let Ok(provider) = app.llama.opencode_provider() else {
        // `o` refuses to open the overlay when there is no block to draw,
        // so this is unreachable in practice — but a modal that draws an
        // empty box rather than nothing at all would be worse than either.
        return;
    };

    let popup = centered(area, desired_width(&provider), 0);
    let width = popup.width.saturating_sub(2) as usize;

    // Height is the one dimension eliding cannot save: a JSON block with
    // a middle missing is not a block, so a box too tall for the terminal
    // says so and points at the key that takes it away whole, rather than
    // drawing an opening brace and running out of rows.
    let lines = match fits(&provider, width, area.height) {
        true => lines(&provider, width),
        false => too_short(&provider, width),
    };

    let popup = centered(
        area,
        popup.width,
        (lines.len() as u16).saturating_add(2).min(area.height),
    );

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).style(Theme::normal()).block(
            Block::default()
                .title(" OpenCode provider ")
                .borders(Borders::ALL)
                .border_style(Theme::border()),
        ),
        popup,
    );
}

/// The rows of the box, cut to `width`.
///
/// Everything is elided rather than clipped, and marked when it is: a
/// JSON line cut at the border reads as a line that ends there, which for
/// a block someone is about to paste is the worst way to be wrong.
fn lines(provider: &opencode::Provider, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::styled(
            indented(&format!("add to {}", opencode::CONFIG_PATH), width),
            Theme::logs(),
        ),
        Line::styled(
            indented("— merge into an existing \"provider\" block", width),
            Theme::logs(),
        ),
        Line::from(""),
    ];

    for line in provider.json().lines() {
        lines.push(Line::styled(indented(line, width), Theme::status_ready()));
    }

    lines.push(Line::from(""));
    lines.push(Line::styled(indented(FOOTER, width), Theme::logs()));
    lines
}

/// Whether the whole block, its two borders included, has room in a
/// terminal `height` rows tall.
fn fits(provider: &opencode::Provider, width: usize, height: u16) -> bool {
    lines(provider, width).len().saturating_add(2) <= height as usize
}

/// What the box says instead when it cannot show the block.
///
/// The block is still copyable — `y` is handled by the mode, not by what
/// is drawn — so the answer to a short terminal is to say how many rows
/// it would need and that the text is one keystroke away regardless.
/// Showing the first eight lines of JSON and stopping would be worse:
/// nothing would say the rest existed.
fn too_short(provider: &opencode::Provider, width: usize) -> Vec<Line<'static>> {
    let needed = lines(provider, width).len() + 2;

    vec![
        Line::from(""),
        Line::styled(
            indented(
                &format!("the provider block needs {needed} rows to show whole"),
                width,
            ),
            Theme::status_starting(),
        ),
        Line::styled(
            indented("a taller terminal shows it — or take it as it is:", width),
            Theme::logs(),
        ),
        Line::from(""),
        Line::styled(indented(FOOTER, width), Theme::logs()),
    ]
}

/// The overlay's own keys. Written here rather than looked up in
/// `keys.rs` because that table documents *Browse* bindings per screen,
/// and these two are neither — they exist only while the box is open, in
/// the same way `[y] yes` on a confirm prompt does.
const FOOTER: &str = "y copy and close · any other key close";

/// An indented line, cut to the box. The indent is cut too: below its own
/// width even two spaces overflow.
fn indented(text: &str, width: usize) -> String {
    components::truncate(&format!("{}{text}", " ".repeat(INDENT)), width)
}

/// The width at which no line has to be cut: the longest one, indented,
/// plus the borders and a space before the right one. `centered` clamps
/// it to the terminal.
fn desired_width(provider: &opencode::Provider) -> u16 {
    let json = provider.json();
    let longest = json
        .lines()
        .map(|line| line.chars().count())
        .chain([
            format!("add to {}", opencode::CONFIG_PATH).chars().count(),
            FOOTER.chars().count(),
        ])
        .max()
        .unwrap_or(0);

    (INDENT + longest + 3) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::llama::caps::Trait;
    use crate::services::llama::opencode::Provider;

    fn provider() -> Provider {
        Provider::from_launch(
            "http://127.0.0.1:1234",
            "gemma4-12b",
            &[
                "--jinja".to_string(),
                "--ctx-size".to_string(),
                "32768".to_string(),
                "--alias".to_string(),
                "gemma4-12b".to_string(),
            ],
            &[],
        )
    }

    fn text(width: usize) -> String {
        lines(&provider(), width)
            .iter()
            .map(|line| line.spans.iter().map(|span| span.content.clone()).collect())
            .collect::<Vec<String>>()
            .join("\n")
    }

    /// The block, where it goes, and how to take it away — the three
    /// things the overlay exists for.
    #[test]
    fn the_overlay_carries_the_block_its_path_and_the_copy_key() {
        let text = text(desired_width(&provider()) as usize);

        assert!(text.contains(opencode::CONFIG_PATH), "{text}");
        assert!(text.contains(opencode::NPM), "{text}");
        assert!(text.contains("http://127.0.0.1:1234/v1"), "{text}");
        assert!(text.contains("gemma4-12b"), "{text}");
        assert!(text.contains("y copy"), "{text}");
    }

    /// At the width it asks for, the JSON is whole — which is the whole
    /// point, since a block with an elision in it will not parse.
    #[test]
    fn nothing_is_cut_at_the_width_the_box_asked_for() {
        let width = desired_width(&provider()) as usize;
        assert!(!text(width.saturating_sub(2)).contains('…'));
    }

    /// A block with every optional field is the tallest one there is, and
    /// it is the case a short terminal has to survive: eliding cannot
    /// help with height, so the box has to notice and say so.
    #[test]
    fn a_terminal_too_short_for_the_block_says_so_rather_than_cutting_it() {
        // Every field present: tool_call, reasoning, attachment, limit.
        let full = Provider::from_launch(
            "http://127.0.0.1:1234",
            "gemma4-12b",
            &[
                "--jinja".to_string(),
                "--ctx-size".to_string(),
                "32768".to_string(),
                "--alias".to_string(),
                "gemma4-12b".to_string(),
                "--reasoning".to_string(),
                "off".to_string(),
            ],
            &[Trait {
                capability: crate::services::llama::caps::Capability::Vision,
                enabled: true,
                detail: None,
            }],
        );
        let width = desired_width(&full) as usize - 2;

        assert!(fits(&full, width, 40), "40 rows is room enough");
        assert!(!fits(&full, width, 24), "an 80x24 terminal is not");

        let text: String = too_short(&full, width)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.clone())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("rows"), "the height is not named: {text}");
        assert!(text.contains("y copy"), "no way out is offered: {text}");
    }

    /// On any narrower terminal a line must still fit its box and must
    /// say it was cut, rather than being clipped by the terminal — the
    /// same rule as `Columns::for_width` and `screen_hint_within`.
    #[test]
    fn a_narrow_box_elides_visibly_and_never_overflows() {
        let full = desired_width(&provider()) as usize;

        for width in 0..=full {
            for line in lines(&provider(), width) {
                let text: String = line.spans.iter().map(|span| span.content.clone()).collect();
                assert!(
                    text.chars().count() <= width,
                    "a {}-character row in a {width}-wide box: {text}",
                    text.chars().count()
                );
            }
        }
    }
}
