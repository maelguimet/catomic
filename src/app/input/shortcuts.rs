//! Purpose: dispatch canonical editor actions after surface and scoped keymap routing.
//! Owns: inline-clanker entry, undo/redo aliases, and central action delegation.
//! Must not: handle printable insertion, cursor movement, raw surfaces, or terminal decoding.
//! Invariants: guarded shortcuts precede editing; actions reuse the central catalog paths.
//! Phase: bounded post-beta input-routing cleanup.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::help_catalog;

use super::super::{hooks, inline_clanker, undo_redo, App};

pub(super) fn handle_inline_clanker_key(
    app: &mut App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    match key {
        KeyEvent {
            code: KeyCode::F(3),
            modifiers: KeyModifiers::SHIFT,
            ..
        } => inline_clanker::clear_changes(app, out)?,
        KeyEvent {
            code: KeyCode::F(3),
            modifiers: KeyModifiers::NONE,
            ..
        } => hooks::before_inline_clanker(app, out)?,
        _ => return Ok(false),
    }
    Ok(true)
}

pub(super) fn handle_key(app: &mut App, out: &mut dyn Write, key: KeyEvent) -> io::Result<bool> {
    if let Some(action) = undo_redo::action_for_key(key) {
        match action {
            undo_redo::HistoryAction::Undo => app.buffer.undo(),
            undo_redo::HistoryAction::Redo => app.buffer.redo(),
        }
        super::finish_content_edit(app, out)?;
        return Ok(true);
    }
    let Some(action) = help_catalog::default_editor_action(key) else {
        return Ok(false);
    };
    super::dispatch_editor_action(app, out, action)?;
    Ok(true)
}
