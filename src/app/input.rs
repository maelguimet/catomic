//! Key input routing for the App goblin loop (Phase 2-ak hygiene extraction).
//!
//! Purpose: own the concrete key dispatch (handle_key / handle_key_with) so
//!   src/app/mod.rs stays focused on App state, run loop, and render.
//! Owns: the big match on KeyEvent; thin delegation from App methods.
//! Must not: introduce command enums/dispatchers; change any key semantics
//!   (Ctrl+Q dirty confirm, Ctrl+S save arm, Ctrl+R reload arm, edit clears
//!   pendings+message, movement does not clear, undo/redo dirty unchanged);
//!   know terminal raw details or Project/LLM.
//! Invariants: identical observable behavior for all documented key paths;
//!   no new public surface on App; extraction only.
//! Phase: 2-ak (file hygiene to address >500 line smell without architecture change).

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::file_state::refresh_dirty;
use super::{reload, save};

/// Thin entry called from the run loop (and a few tests).
pub(crate) fn handle_key(app: &mut super::App, key: KeyEvent) -> io::Result<()> {
    let mut out = io::stdout();
    handle_key_with(app, &mut out, key)
}

/// Route key handling + associated renders through a writer.
/// Smallest seam so tests can capture render side-effects for e.g. Ctrl+Q message.
/// The public-in-module handle_key keeps the run loop and existing calls unchanged.
pub(crate) fn handle_key_with(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<()> {
    match key {
        // Quit (Ctrl+Q)
        // - clean: quit immediately
        // - dirty + !pending: set pending=true + warning message; do NOT quit
        // - dirty + pending: quit (force, without save)
        // Movement keys leave pending/message as-is (simplest behavior; documented).
        // Actual content-mutating edits (insert/delete/undo/redo) clear BOTH pending_confirm and message
        // (so stale quit warnings disappear after typing). Save success also clears them.
        KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => {
            if !app.file.dirty {
                app.should_quit = true;
            } else if app.pending_quit_confirm {
                app.should_quit = true;
            } else {
                app.pending_quit_confirm = true;
                app.message = Some(
                    "Unsaved changes. Press Ctrl+Q again to quit without saving, Ctrl+S to save."
                        .to_string(),
                );
                app.render(out)?;
                // do not quit
            }
        }

        // Save (Ctrl+S) -- thin arm; real logic + guard lives in save module
        // (extracted Phase 2-o to keep this file focused). Semantics unchanged.
        KeyEvent {
            code: KeyCode::Char('s'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => {
            save::handle_save(app, out)?;
        }

        // Manual reload check (Phase 2-s). Thin call; decision + perform logic lives in reload.rs.
        KeyEvent {
            code: KeyCode::Char('r'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => {
            reload::handle_reload_key(app, out)?;
        }

        // Enter produces KeyCode::Enter (not Char('\n')). Handle explicitly.
        // The Char \n/\r check below catches any that might arrive via paste
        // or other terminal paths.
        KeyEvent {
            code: KeyCode::Enter,
            ..
        } => {
            app.buffer.insert_newline();
            refresh_dirty(&mut app.file, &*app.buffer);
            app.pending_quit_confirm = false;
            app.pending_save_conflict = None;
            app.pending_reload = None;
            app.message = None;
            app.reveal_cursor();
            app.render(out)?;
        }

        // Undo / Redo (Phase 1C). Ctrl+Z undo; Ctrl+Y and Ctrl+Shift+Z redo.
        // Redo must handle both common terminal reports for Ctrl+Shift+Z:
        //   - KeyCode::Char('z') + CONTROL + SHIFT
        //   - KeyCode::Char('Z') + CONTROL + SHIFT
        // Place before generic Char so CONTROL combos fire. No other UI changes.
        // Dirty is computed exactly from edit_history_position vs saved token (Phase 2-j).
        KeyEvent {
            code: KeyCode::Char('z'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL)
            && !modifiers.contains(KeyModifiers::SHIFT) =>
        {
            app.buffer.undo();
            refresh_dirty(&mut app.file, &*app.buffer);
            app.pending_quit_confirm = false;
            app.pending_save_conflict = None;
            app.pending_reload = None;
            app.message = None;
            app.reveal_cursor();
            app.render(out)?;
        }
        KeyEvent {
            code: KeyCode::Char('z'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL)
            && modifiers.contains(KeyModifiers::SHIFT) =>
        {
            app.buffer.redo();
            refresh_dirty(&mut app.file, &*app.buffer);
            app.pending_quit_confirm = false;
            app.pending_save_conflict = None;
            app.pending_reload = None;
            app.message = None;
            app.reveal_cursor();
            app.render(out)?;
        }
        KeyEvent {
            code: KeyCode::Char('Z'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL)
            && modifiers.contains(KeyModifiers::SHIFT) =>
        {
            app.buffer.redo();
            refresh_dirty(&mut app.file, &*app.buffer);
            app.pending_quit_confirm = false;
            app.pending_save_conflict = None;
            app.pending_reload = None;
            app.message = None;
            app.reveal_cursor();
            app.render(out)?;
        }
        KeyEvent {
            code: KeyCode::Char('y'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL) => {
            app.buffer.redo();
            refresh_dirty(&mut app.file, &*app.buffer);
            app.pending_quit_confirm = false;
            app.pending_save_conflict = None;
            app.pending_reload = None;
            app.message = None;
            app.reveal_cursor();
            app.render(out)?;
        }

        // Basic movement + editing (Phase 0)
        // Accept any Char that is not control. Apply SHIFT modifier for
        // uppercase letters (crossterm may report lowercase + SHIFT).
        // Specific Ctrl+S / Ctrl+Q arms above take precedence for CONTROL.
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            ..
        } => {
            if modifiers.contains(KeyModifiers::CONTROL) {
                // Other Ctrl+letter combos ignored in Phase 0
            } else if c == '\n' || c == '\r' {
                app.buffer.insert_newline();
                refresh_dirty(&mut app.file, &*app.buffer);
                app.pending_quit_confirm = false;
                app.pending_save_conflict = None;
                app.pending_reload = None;
                app.message = None;
            } else if !c.is_control() {
                let ch = if modifiers.contains(KeyModifiers::SHIFT) && c.is_ascii_lowercase() {
                    c.to_ascii_uppercase()
                } else {
                    c
                };
                app.buffer.insert_char(ch);
                refresh_dirty(&mut app.file, &*app.buffer);
                app.pending_quit_confirm = false;
                app.pending_save_conflict = None;
                app.pending_reload = None;
                app.message = None;
            }
            app.reveal_cursor();
            app.render(out)?;
        }

        KeyEvent {
            code: KeyCode::Backspace,
            ..
        } => {
            app.buffer.delete_back();
            refresh_dirty(&mut app.file, &*app.buffer);
            app.pending_quit_confirm = false;
            app.pending_save_conflict = None;
            app.pending_reload = None;
            app.message = None;
            app.reveal_cursor();
            app.render(out)?;
        }

        KeyEvent {
            code: KeyCode::Delete,
            ..
        } => {
            app.buffer.delete_forward();
            refresh_dirty(&mut app.file, &*app.buffer);
            app.pending_quit_confirm = false;
            app.pending_save_conflict = None;
            app.pending_reload = None;
            app.message = None;
            app.reveal_cursor();
            app.render(out)?;
        }

        KeyEvent {
            code: KeyCode::Left,
            ..
        } => {
            app.buffer.move_left();
            app.reveal_cursor();
            app.render(out)?;
        }

        KeyEvent {
            code: KeyCode::Right,
            ..
        } => {
            app.buffer.move_right();
            app.reveal_cursor();
            app.render(out)?;
        }

        KeyEvent {
            code: KeyCode::Up, ..
        } => {
            app.buffer.move_up();
            app.reveal_cursor();
            app.render(out)?;
        }

        KeyEvent {
            code: KeyCode::Down,
            ..
        } => {
            app.buffer.move_down();
            app.reveal_cursor();
            app.render(out)?;
        }

        _ => {}
    }

    Ok(())
}
