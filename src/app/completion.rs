//! Purpose: connect bounded current-buffer word completion to explicit editor input.
//! Owns: transient candidate selection, key handling, messages, and atomic acceptance.
//! Must not: scan projects/buffers, start discovery, spawn work/processes, or emit terminal codes.
//! Invariants: no content changes before Enter; accepted text is one undoable replacement.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent};

use crate::buffer::Cursor;
use crate::config::actions::Action;
mod candidates;
use candidates::PREFIX_COLS;

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
    app.completion.active.take().is_some()
}

fn open(app: &mut super::App, out: &mut dyn Write) -> io::Result<OpenOutcome> {
    if super::view::is_preview(app) || app.buffer.is_read_only() {
        app.message_info("Local completion requires an editable source buffer.");
        app.render(out)?;
        return Ok(OpenOutcome::Handled);
    }
    if app.selection.active().is_some() {
        app.message_info("Dismiss the selection before completing a word.");
        app.render(out)?;
        return Ok(OpenOutcome::Handled);
    }

    let cursor = app.buffer.cursor();
    let Some(prefix) = candidates::read_prefix(app, cursor)? else {
        app.message_info(format!(
            "Completion prefix exceeds the {PREFIX_COLS}-column limit."
        ));
        app.render(out)?;
        return Ok(OpenOutcome::Handled);
    };
    if prefix.text.is_empty() {
        app.message_info("Type a word prefix before requesting completion.");
        app.render(out)?;
        return Ok(OpenOutcome::NoCandidate);
    }
    let completion_candidates = candidates::read_candidates(app, cursor, &prefix)?;
    if completion_candidates.is_empty() {
        app.message_info(format!("No completion for '{}'.", prefix.text));
        app.render(out)?;
        return Ok(OpenOutcome::NoCandidate);
    }
    let start = Cursor {
        row: cursor.row,
        col: cursor.col.saturating_sub(prefix.text.chars().count()),
    };
    app.completion.active = Some(ActiveCompletion {
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

pub(crate) fn trigger(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    open(app, out).map(|_| ())
}

pub(crate) fn dispatch_action(
    app: &mut super::App,
    out: &mut dyn Write,
    action: Action,
) -> io::Result<bool> {
    if !is_active(app) {
        return Ok(false);
    }
    match action {
        Action::CompletionNext => cycle(app, true),
        Action::CompletionPrevious => cycle(app, false),
        Action::CompletionAccept => return accept(app, out).map(|()| true),
        Action::CompletionCancel => {
            cancel(app);
            app.message = None;
        }
        _ => return Ok(false),
    }
    update_message_unless_dismissed(app);
    app.render(out)?;
    Ok(true)
}

pub(crate) fn dispatch_editor_action(
    app: &mut super::App,
    out: &mut dyn Write,
    action: Action,
) -> io::Result<bool> {
    if action != Action::Indent {
        return Ok(false);
    }
    Ok(open(app, out)? == OpenOutcome::Handled)
}

fn handle_active_key(app: &mut super::App, out: &mut dyn Write, key: KeyEvent) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc => {
            cancel(app);
            app.message = None;
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
    let active = app.completion.active.as_mut().expect("active completion");
    let count = active.candidates.len();
    active.selected = if forward {
        active.selected.saturating_add(1) % count
    } else {
        active.selected.saturating_add(count - 1) % count
    };
}

fn accept(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let active = app.completion.active.take().expect("active completion");
    let unchanged = app.buffer.cursor() == active.end
        && app.buffer.text_range(active.start, active.end)? == active.prefix;
    if !unchanged {
        app.message_info("Completion dismissed because the prefix changed.");
        return app.render(out);
    }
    let candidate = &active.candidates[active.selected];
    app.buffer
        .replace_range(active.start, active.end, candidate)?;
    super::input::finish_content_edit(app, out)
}

fn update_message(app: &mut super::App) {
    let active = app.completion.active.as_ref().expect("active completion");
    app.message_info(format!(
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

pub(super) fn is_active(app: &super::App) -> bool {
    app.completion.active.is_some()
}

fn is_trigger(key: KeyEvent) -> bool {
    key.code == KeyCode::Tab
}

fn is_cycle_forward(key: KeyEvent) -> bool {
    is_trigger(key)
}

#[cfg(test)]
mod tests;
