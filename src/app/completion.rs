//! Purpose: connect bounded local and cached Project completion to explicit editor input.
//! Owns: transient candidate selection, key handling, messages, and atomic acceptance.
//! Must not: scan projects/buffers, start discovery, spawn work/processes, or emit terminal codes.
//! Invariants: no content changes before Enter; accepted text is one undoable replacement.
//! Phase: 5-a local completion through 5-e Project path completion.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::Cursor;
mod candidates;
use candidates::{CandidateRead, PREFIX_COLS};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenOutcome {
    Handled,
    NoCandidate,
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
    let outcome = open(app, out)?;
    Ok(outcome == OpenOutcome::Handled || key.code != KeyCode::Tab)
}

pub(crate) fn cancel(app: &mut super::App) -> bool {
    if let Some(state) = app.completion.as_mut() {
        return state.active.take().is_some();
    }
    false
}

fn open(app: &mut super::App, out: &mut dyn Write) -> io::Result<OpenOutcome> {
    if !app.caps.local_completion || app.completion.is_none() {
        app.message = Some("Local completion is unavailable.".to_string());
        app.render(out)?;
        return Ok(OpenOutcome::Handled);
    }
    if super::view::is_preview(app)
        || super::lint::is_viewing(app)
        || super::project_files::is_viewing(app)
        || app.buffer.is_read_only()
    {
        app.message = Some("Local completion requires an editable source buffer.".to_string());
        app.render(out)?;
        return Ok(OpenOutcome::Handled);
    }
    if app.selection.active().is_some() {
        app.message = Some("Dismiss the selection before completing a word.".to_string());
        app.render(out)?;
        return Ok(OpenOutcome::Handled);
    }

    let cursor = app.buffer.cursor();
    let Some(prefix) = candidates::read_prefix(app, cursor)? else {
        app.message = Some(format!(
            "Completion prefix exceeds the {PREFIX_COLS}-column limit."
        ));
        app.render(out)?;
        return Ok(OpenOutcome::Handled);
    };
    if prefix.text.is_empty() {
        app.message = Some("Type a word prefix before requesting completion.".to_string());
        app.render(out)?;
        return Ok(OpenOutcome::NoCandidate);
    }
    let completion_candidates = match candidates::read_candidates(app, cursor, &prefix)? {
        CandidateRead::Ready(candidates) => candidates,
        CandidateRead::ProjectFilesUnavailable => {
            app.message = Some("Run :files before requesting Project path completion.".to_string());
            app.render(out)?;
            return Ok(OpenOutcome::Handled);
        }
    };
    if completion_candidates.is_empty() {
        app.message = Some(format!("No completion for '{}'.", prefix.text));
        app.render(out)?;
        return Ok(OpenOutcome::NoCandidate);
    }
    let start = Cursor {
        row: cursor.row,
        col: cursor.col.saturating_sub(prefix.text.chars().count()),
    };
    app.completion.as_mut().expect("capability checked").active = Some(ActiveCompletion {
        prefix: prefix.text,
        start,
        end: cursor,
        candidates: completion_candidates,
        selected: 0,
    });
    update_message(app);
    app.render(out)?;
    Ok(OpenOutcome::Handled)
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
