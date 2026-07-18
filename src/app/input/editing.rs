//! Purpose: dispatch canonical normal-mode editing and movement keys.
//! Owns: indentation, newline, insertion, deletion, and arrow movement routing.
//! Must not: handle active surfaces, translate keybindings, save files, or decode terminal bytes.
//! Invariants: edits use common cleanup; movement preserves pending confirmations and messages.
//! Phase: bounded post-beta input-routing cleanup.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::{indentation, navigation, overwrite, App};

pub(super) fn handle_key(app: &mut App, out: &mut dyn Write, key: KeyEvent) -> io::Result<()> {
    match key {
        KeyEvent {
            code: KeyCode::Tab,
            modifiers: KeyModifiers::NONE,
            ..
        } => indentation::handle_tab(app, out, false)?,
        KeyEvent {
            code: KeyCode::BackTab,
            ..
        }
        | KeyEvent {
            code: KeyCode::Tab,
            modifiers: KeyModifiers::SHIFT,
            ..
        } => indentation::handle_tab(app, out, true)?,
        KeyEvent {
            code: KeyCode::Enter,
            ..
        } => indentation::insert_newline(app, out)?,
        KeyEvent {
            code: KeyCode::Char(character),
            modifiers,
            ..
        } => handle_character(app, out, character, modifiers)?,
        KeyEvent {
            code: KeyCode::Backspace,
            ..
        } => navigation::delete_grapheme(app, out, false)?,
        KeyEvent {
            code: KeyCode::Delete,
            ..
        } => navigation::delete_grapheme(app, out, true)?,
        KeyEvent {
            code: KeyCode::Left,
            ..
        } => move_horizontal(app, out, false)?,
        KeyEvent {
            code: KeyCode::Right,
            ..
        } => move_horizontal(app, out, true)?,
        KeyEvent {
            code: KeyCode::Up, ..
        } => move_vertical(app, out, false)?,
        KeyEvent {
            code: KeyCode::Down,
            ..
        } => move_vertical(app, out, true)?,
        _ => {}
    }
    Ok(())
}

fn handle_character(
    app: &mut App,
    out: &mut dyn Write,
    character: char,
    modifiers: KeyModifiers,
) -> io::Result<()> {
    if modifiers.contains(KeyModifiers::CONTROL) {
        app.reveal_cursor();
        return app.render(out);
    }
    if character == '\n' || character == '\r' {
        return indentation::insert_newline(app, out);
    }
    if character.is_control() {
        app.reveal_cursor();
        return app.render(out);
    }
    let character = if modifiers.contains(KeyModifiers::SHIFT) && character.is_ascii_lowercase() {
        character.to_ascii_uppercase()
    } else {
        character
    };
    overwrite::type_char(app, character)?;
    super::finish_content_edit(app, out)
}

fn move_horizontal(app: &mut App, out: &mut dyn Write, forward: bool) -> io::Result<()> {
    app.selection.clear();
    navigation::move_grapheme(app, forward)?;
    app.reveal_cursor();
    app.render(out)
}

fn move_vertical(app: &mut App, out: &mut dyn Write, forward: bool) -> io::Result<()> {
    app.selection.clear();
    if forward {
        app.buffer.move_down();
    } else {
        app.buffer.move_up();
    }
    navigation::snap_current_grapheme(app)?;
    app.reveal_cursor();
    app.render(out)
}
