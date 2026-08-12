//! The `:help` overlay: every command the `:` bar accepts.
//!
//! The counterpart to the `?` overlay, which does the same for keys, and
//! read entirely from `commands.rs` for the same reason — a command cannot
//! appear here without being dispatched, nor be dispatched without
//! appearing here.
//!
//! It is an overlay rather than a line printed into the log, which is what
//! `:help` used to do. The log is on another screen, is where the server
//! also writes hundreds of lines during a load, and scrolls; asking "what
//! can I type here" and being answered somewhere else, later, is not an
//! answer.

use crate::{app::App, commands, components::centered, theme::Theme};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, _app: &App, area: Rect) {
    // Sized from the content rather than to a round number: the usage
    // column is as wide as the widest usage (`:router [--max N] [--idle S]`
    // is 28 characters and used to run straight into its own summary), and
    // the box asks for whatever the longest line then needs. `centered`
    // clamps it to the terminal, and the summaries are elided — visibly —
    // to whatever is left.
    let popup = centered(area, desired_width(), 0);
    let lines = lines(popup.width.saturating_sub(2) as usize);
    let popup = centered(
        area,
        popup.width,
        (lines.len() as u16).saturating_add(2).min(area.height),
    );

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).style(Theme::normal()).block(
            Block::default()
                .title(" Commands ")
                .borders(Borders::ALL)
                .border_style(Theme::border()),
        ),
        popup,
    );
}

/// Two spaces of indent, as every other listing here uses.
const INDENT: usize = 2;
/// The gap between a usage and its summary.
const GAP: usize = 1;

/// The widest `:usage` in the table, so nothing collides with its own
/// description. Measured rather than guessed at, because the answer
/// changes the moment a command grows an argument.
fn usage_width() -> usize {
    commands::visible()
        .map(|command| command.usage.chars().count() + 1)
        .max()
        .unwrap_or(0)
}

/// The width at which no row has to be cut: the longest complete row.
fn content_width() -> usize {
    let longest = commands::visible()
        .map(|command| command.summary.chars().count())
        .max()
        .unwrap_or(0);

    INDENT + usage_width() + GAP + longest
}

/// How wide the box would like to be. `centered` clamps it to the
/// terminal, and `lines` elides to whatever survives.
fn desired_width() -> u16 {
    // the content, its two borders, and a space before the right one
    (content_width() + 3) as u16
}

fn lines(width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for group in commands::Group::ALL {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::styled(
            indented(group.label(), width),
            Theme::command(),
        ));
        lines.extend(commands::in_group(group).map(|command| row(command, width)));
    }

    lines.push(Line::from(""));
    lines.push(Line::styled(
        indented("esc close · ? lists the keys instead", width),
        Theme::logs(),
    ));
    lines
}

/// An indented line, cut to the box. The indent is cut too: at a width
/// below it, even two spaces overflow.
fn indented(text: &str, width: usize) -> String {
    truncate(&format!("{}{text}", " ".repeat(INDENT)), width)
}

fn row(command: &commands::Command, width: usize) -> Line<'static> {
    // On a terminal narrower than the usage column itself, the usage is
    // what survives: a command you cannot type is not worth describing.
    let column = usage_width().min(width.saturating_sub(INDENT));
    let usage = truncate(&format!(":{}", command.usage), column);
    let head = truncate(
        &format!("{}{usage:<column$}", " ".repeat(INDENT)),
        width.min(INDENT + column),
    );
    let room = width.saturating_sub(head.chars().count() + GAP);

    Line::from(vec![
        Span::styled(head, Theme::status_ready()),
        Span::styled(
            match room {
                0 => String::new(),
                room => format!("{}{}", " ".repeat(GAP), truncate(command.summary, room)),
            },
            Theme::normal(),
        ),
    ])
}

/// Cuts a summary to the room there is, and says that it did.
///
/// Letting the terminal clip it would read as a summary that simply ends
/// there — the same dishonesty `Columns::for_width` and
/// `keys::screen_hint_within` exist to avoid.
fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }

    let kept: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The listing with room for everything, which is what the assertions
    /// about content are about; the narrow case is its own test.
    fn text() -> String {
        lines(content_width())
            .iter()
            .map(|line| line.spans.iter().map(|span| span.content.clone()).collect())
            .collect::<Vec<String>>()
            .join("\n")
    }

    /// The overlay is the only place the commands are written down for a
    /// reader, so every one of them has to reach it — with its arguments,
    /// since `ping` without a model is a usage error rather than a command.
    #[test]
    fn every_visible_command_is_listed_with_its_usage() {
        let text = text();

        for command in commands::visible() {
            assert!(
                text.contains(command.usage),
                "{} is not in the listing",
                command.name
            );
            assert!(
                text.contains(command.summary),
                "{} has no summary",
                command.name
            );
        }
    }

    /// `launch!` skips the port-in-use check. It is emitted by the
    /// confirmation prompt, and putting it in front of the user would only
    /// invite skipping a check that exists for a reason.
    #[test]
    fn the_forced_launch_is_not_advertised() {
        assert!(!text().contains("launch!"));
    }

    #[test]
    fn every_group_gets_a_heading() {
        let text = text();
        for group in commands::Group::ALL {
            assert!(text.contains(group.label()), "{:?} heading", group.label());
        }
    }

    /// At its own width every summary is whole — the box asks for exactly
    /// as much as the longest row needs.
    #[test]
    fn nothing_is_elided_when_the_box_gets_the_width_it_asked_for() {
        assert!(!text().contains('…'));
    }

    /// On a narrower terminal a row must still fit its box, and must say
    /// that something was cut rather than letting the terminal clip it —
    /// a clipped summary reads as one that simply ended there.
    #[test]
    fn a_narrow_box_elides_visibly_and_never_overflows() {
        for width in 0..=content_width() {
            let mut shortened = false;

            for line in lines(width) {
                let text: String = line.spans.iter().map(|span| span.content.clone()).collect();
                assert!(
                    text.chars().count() <= width,
                    "a {}-character row in a {width}-wide box: {text}",
                    text.chars().count()
                );
                shortened |= text.contains('…');
            }

            // A box with no columns at all cannot carry the marker either;
            // anything narrower than the content and wider than nothing
            // must say that it cut something.
            assert!(
                shortened || width == content_width() || width == 0,
                "nothing was elided at {width} and nothing said so"
            );
        }
    }
}
