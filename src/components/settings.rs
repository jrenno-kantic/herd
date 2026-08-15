//! The Settings screen: every `[server]`, `[*]` and per-model key, with
//! the ini value and any session override shown side by side so it is
//! always clear what was diverged from.

use crate::{
    app::{App, Mode, Screen, SettingRow},
    components, keys, layout,
    services::llama::overrides,
    theme::Theme,
};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

/// Same treatment as the Models table: a `List`, so a preset with more
/// keys than the terminal has rows can still be scrolled through instead
/// of moving an invisible cursor past the bottom edge.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = block().title_top(
        Line::styled(
            components::position(
                app.llama.settings_cursor,
                app.llama.setting_entry_indices().len(),
            ),
            Theme::border(),
        )
        .right_aligned(),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = app.llama.setting_rows();

    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new("no config loaded").style(Theme::logs()),
            inner,
        );
        return;
    }

    let chunks = layout::rows_with_footer(inner, 1);

    frame.render_widget(
        Paragraph::new(format!(
            "  {}",
            footer(app, components::hint_width(chunks.second.width, false, 0))
        ))
        .style(Theme::logs()),
        chunks.second,
    );

    let selected_index = app
        .llama
        .setting_entry_indices()
        .get(app.llama.settings_cursor)
        .copied();

    let width = chunks.first.width as usize;

    // One item per row, so the list's selection index is an index into
    // `rows` — headers included, which is why they are a two-line item
    // rather than two items.
    let mut items: Vec<ListItem> = Vec::new();

    for (index, row) in rows.iter().enumerate() {
        match row {
            SettingRow::Header(text) => {
                items.push(ListItem::new(vec![
                    Line::from(""),
                    Line::styled(
                        components::truncate(&format!("  {text}"), width),
                        Theme::command(),
                    ),
                ]));
            }
            SettingRow::Entry {
                key,
                ini_value,
                override_value,
                ..
            } => {
                let selected = Some(index) == selected_index;
                let editing = selected && app.mode == Mode::EditSetting;

                let effective = override_value.as_deref().unwrap_or(ini_value);
                // A checkbox in front of the value, so a row that responds
                // to Enter by flipping rather than opening an editor looks
                // different before it is pressed.
                let checkbox = match toggle_state(effective) {
                    Some(true) => "[x] ",
                    Some(false) => "[ ] ",
                    None => "",
                };

                let value = if editing {
                    format!("{}▏", app.llama.edit_buffer)
                } else {
                    match override_value {
                        Some(value) => format!("{checkbox}{value}   (ini: {ini_value})"),
                        None => format!("{checkbox}{ini_value}"),
                    }
                };

                let marker = match (selected, override_value.is_some()) {
                    (true, true) => "▸*",
                    (true, false) => "▸ ",
                    (false, true) => " *",
                    (false, false) => "  ",
                };

                // Clipped to the pane, with a mark: a value cut by the
                // terminal instead — `unsloth/gemma-4-12B-it-qat-GGUF:UD-…`
                // at 80 columns — reads as a value that simply ends there,
                // which for a setting is worse than saying nothing.
                items.push(ListItem::new(Line::styled(
                    components::truncate(&format!("{marker}{key:<26} {value}"), width),
                    if selected {
                        Theme::selected()
                    } else if override_value.is_some() {
                        Theme::status_starting()
                    } else {
                        Theme::normal()
                    },
                )));
            }
        }
    }

    let mut state = ListState::default().with_selected(selected_index);
    frame.render_stateful_widget(List::new(items), chunks.first, &mut state);

    // Counted in rows, not items: a section header is drawn as a blank
    // line plus a title, so a bar that treated it as one row would sit
    // a row out for every header above the cursor — up to three here.
    let heights: Vec<usize> = rows
        .iter()
        .map(|row| match row {
            SettingRow::Header(_) => 2,
            SettingRow::Entry { .. } => 1,
        })
        .collect();

    components::tall_list_scrollbar(
        frame,
        chunks.first,
        area.x + area.width.saturating_sub(1),
        &heights,
        selected_index.unwrap_or(0),
    );
}

/// Whether a value is a boolean and, if so, which way it is set.
///
/// `None` for everything else, so only the rows Enter actually flips get a
/// checkbox — a checkbox in front of a port number would promise something
/// pressing Enter does not do.
fn toggle_state(value: &str) -> Option<bool> {
    overrides::toggled(value)?;
    Some(matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "on" | "yes"
    ))
}

/// Names what Enter will do to the row under the cursor. The Settings
/// screen is the one place where that key has two behaviours, so it says
/// which one is armed rather than leaving the user to press it and see.
fn footer(app: &App, width: usize) -> String {
    let toggles = app
        .llama
        .selected_setting()
        .and_then(|row| {
            let (_, _, _, ini_value, override_value) = row.as_entry()?;
            toggle_state(override_value.unwrap_or(ini_value))
        })
        .is_some();

    // The "enter flips this one" note is part of the line, so the hints
    // are fitted to what is left after it rather than to the whole pane.
    const NOTE: &str = "   (enter flips this one)";
    let taken = if toggles { NOTE.len() } else { 0 };

    let hint = keys::screen_hint_within(Screen::Settings, width.saturating_sub(taken));
    if toggles {
        format!("{hint}{NOTE}")
    } else {
        hint
    }
}

/// Where an edit ends up is the standing fact about this screen, so it
/// lives in the title rather than competing with the key hints for the
/// one line at the bottom. Overrides are remembered — in a file herd owns
/// — and the hand-written `models.ini` is still never touched.
fn block() -> Block<'static> {
    Block::default()
        .title(Span::styled(
            " Settings · kept in ~/.herd_config, models.ini is never written ",
            Theme::normal(),
        ))
        .borders(Borders::ALL)
        .border_style(Theme::border())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only the rows Enter actually flips get a checkbox — one in front of
    /// a port number would promise something pressing Enter does not do.
    #[test]
    fn only_booleans_get_a_checkbox() {
        assert_eq!(toggle_state("true"), Some(true));
        assert_eq!(toggle_state("on"), Some(true));
        assert_eq!(toggle_state("yes"), Some(true));
        assert_eq!(toggle_state("false"), Some(false));
        assert_eq!(toggle_state("off"), Some(false));

        for value in ["1234", "0", "1", "auto", "unsloth/x:Q4"] {
            assert_eq!(toggle_state(value), None, "{value:?} got a checkbox");
        }
    }

    #[test]
    fn the_checkbox_ignores_case_and_padding() {
        assert_eq!(toggle_state("  TRUE "), Some(true));
        assert_eq!(toggle_state("Off"), Some(false));
    }
}
