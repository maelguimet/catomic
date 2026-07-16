//! Purpose: connect bounded local word completion to explicit editor input.
//! Owns: transient candidate selection, key handling, messages, and atomic acceptance.
//! Must not: scan an entire buffer, spawn work/processes, access Project state, or emit terminal codes.
//! Invariants: no content changes before Enter; accepted text is one undoable replacement.
//! Phase: 5-a local current-buffer completion.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::Cursor;
use crate::editor::completion::{complete_words, is_word_char, prefix_before_cursor};

const CONTEXT_ROWS: usize = 257;
const CONTEXT_COLS: usize = 1_024;
const PREFIX_COLS: usize = 512;
const MAX_CANDIDATES: usize = 16;

#[derive(Default)]
pub(crate) struct CompletionUiState {
    active: Option<ActiveCompletion>,
}

struct ActiveCompletion {
    prefix: String,
    start: Cursor,
    end: Cursor,
    candidates: Vec<String>,
    selected: usize,
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if is_active(app) {
        return handle_active_key(app, out, key);
    }
    if !is_trigger(key) {
        return Ok(false);
    }
    open(app, out)?;
    Ok(true)
}

pub(crate) fn cancel(app: &mut super::App) -> bool {
    if let Some(state) = app.completion.as_mut() {
        return state.active.take().is_some();
    }
    false
}

fn open(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if !app.caps.local_completion || app.completion.is_none() {
        app.message = Some("Local completion is unavailable.".to_string());
        return app.render(out);
    }
    if super::view::is_preview(app) || app.buffer.is_read_only() {
        app.message = Some("Local completion requires an editable source buffer.".to_string());
        return app.render(out);
    }
    if app.selection.active().is_some() {
        app.message = Some("Dismiss the selection before completing a word.".to_string());
        return app.render(out);
    }

    let cursor = app.buffer.cursor();
    let Some(prefix) = read_prefix(app, cursor)? else {
        app.message = Some(format!(
            "Word prefix exceeds the {PREFIX_COLS}-column completion limit."
        ));
        return app.render(out);
    };
    if prefix.is_empty() {
        app.message = Some("Type a word prefix before requesting completion.".to_string());
        return app.render(out);
    }
    let candidates = read_candidates(app, cursor, &prefix)?;
    if candidates.is_empty() {
        app.message = Some(format!("No local completion for '{prefix}'."));
        return app.render(out);
    }
    let start = Cursor {
        row: cursor.row,
        col: cursor.col.saturating_sub(prefix.chars().count()),
    };
    app.completion.as_mut().expect("capability checked").active = Some(ActiveCompletion {
        prefix,
        start,
        end: cursor,
        candidates,
        selected: 0,
    });
    update_message(app);
    app.render(out)
}

fn read_prefix(app: &super::App, cursor: Cursor) -> io::Result<Option<String>> {
    let start_col = cursor.col.saturating_sub(PREFIX_COLS);
    let read_start = start_col.saturating_sub(1);
    let relative_cursor = cursor.col.saturating_sub(read_start);
    let line = app
        .buffer
        .try_visible_lines_window(cursor.row, 1, read_start, relative_cursor)?
        .into_iter()
        .next()
        .map(|line| line.content)
        .unwrap_or_default();
    let prefix = prefix_before_cursor(&line, relative_cursor);
    if read_start < start_col && prefix.chars().count() == relative_cursor {
        return Ok(None);
    }
    Ok(Some(prefix))
}

fn read_candidates(app: &super::App, cursor: Cursor, prefix: &str) -> io::Result<Vec<String>> {
    let row_start = cursor.row.saturating_sub(CONTEXT_ROWS / 2);
    let col_start = cursor.col.saturating_sub(CONTEXT_COLS / 2);
    let lines = app.buffer.try_visible_lines_window(
        row_start,
        CONTEXT_ROWS,
        col_start,
        CONTEXT_COLS + 1,
    )?;
    let fragments: Vec<String> = lines
        .iter()
        .map(|line| complete_fragment(&line.content, col_start))
        .collect();
    Ok(complete_words(
        fragments.iter().map(String::as_str),
        prefix,
        MAX_CANDIDATES,
    ))
}

fn complete_fragment(content: &str, start_col: usize) -> String {
    let mut chars: Vec<char> = content.chars().collect();
    let trailing_cut = chars.len() > CONTEXT_COLS;
    chars.truncate(CONTEXT_COLS);
    let start = if start_col == 0 {
        0
    } else {
        chars
            .iter()
            .position(|ch| !is_word_char(*ch))
            .map_or(chars.len(), |index| index + 1)
    };
    let end = if trailing_cut {
        chars.iter().rposition(|ch| !is_word_char(*ch)).unwrap_or(0)
    } else {
        chars.len()
    };
    chars[start.min(end)..end].iter().collect()
}

fn handle_active_key(app: &mut super::App, out: &mut dyn Write, key: KeyEvent) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc => {
            cancel(app);
            app.message = Some("Completion dismissed.".to_string());
        }
        KeyCode::Enter => return accept(app, out).map(|()| true),
        KeyCode::BackTab => cycle(app, false),
        _ if is_cycle_forward(key) => cycle(app, true),
        _ => {
            if cancel(app) {
                app.message = None;
            }
            return Ok(false);
        }
    }
    update_message_unless_dismissed(app);
    app.render(out)?;
    Ok(true)
}

fn cycle(app: &mut super::App, forward: bool) {
    let active = app
        .completion
        .as_mut()
        .and_then(|state| state.active.as_mut())
        .expect("active completion");
    let count = active.candidates.len();
    active.selected = if forward {
        active.selected.saturating_add(1) % count
    } else {
        active.selected.saturating_add(count - 1) % count
    };
}

fn accept(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let active = app
        .completion
        .as_mut()
        .and_then(|state| state.active.take())
        .expect("active completion");
    let unchanged = app.buffer.cursor() == active.end
        && app.buffer.text_range(active.start, active.end)? == active.prefix;
    if !unchanged {
        app.message = Some("Completion dismissed because the prefix changed.".to_string());
        return app.render(out);
    }
    let candidate = &active.candidates[active.selected];
    app.buffer
        .replace_range(active.start, active.end, candidate)?;
    super::input::finish_content_edit(app, out)
}

fn update_message(app: &mut super::App) {
    let active = app
        .completion
        .as_ref()
        .and_then(|state| state.active.as_ref())
        .expect("active completion");
    app.message = Some(format!(
        "Completion {}/{}: {} (Tab next, Enter accept, Esc dismiss)",
        active.selected + 1,
        active.candidates.len(),
        active.candidates[active.selected]
    ));
}

fn update_message_unless_dismissed(app: &mut super::App) {
    if is_active(app) {
        update_message(app);
    }
}

fn is_active(app: &super::App) -> bool {
    app.completion
        .as_ref()
        .is_some_and(|state| state.active.is_some())
}

fn is_trigger(key: KeyEvent) -> bool {
    key.code == KeyCode::Tab || is_control_space(key)
}

fn is_cycle_forward(key: KeyEvent) -> bool {
    key.code == KeyCode::Tab || is_control_space(key)
}

fn is_control_space(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char(' ') | KeyCode::Null)
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests;
