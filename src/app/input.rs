//! Purpose: route normalized key and paste events into explicit App/editor actions.
//! Owns: key precedence, ordinary edit dispatch, and common post-edit cleanup.
//! Must not: decode raw terminal bytes, access buffer internals, render content, or network.
//! Invariants: active local surfaces handle raw keys before normal-mode overrides;
//!   guarded shortcuts win over text input; one user edit clears stale confirmations.
//! Phase: 3-d keyboard selection and bracketed-paste integration.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::file_state::refresh_dirty;
use super::{
    buffers, command_prompt, completion, external_command, help, indentation, lint, llm_answer,
    llm_preview, llm_request, navigation, paging, project_files, recovery, reload, replace,
    repo_llm, save, search, selection, view,
};

/// Common post-content-mutation cleanup used by insert, delete, newline, undo, redo paths.
/// Centralizes the exact sequence that must run after any buffer-mutating key:
/// refresh dirty from history token, clear all transient pending confirmations and
/// messages, reveal cursor, and render. Movement paths deliberately do not call this.
/// Behavior must remain identical to the prior inlined blocks (including no-op undo/redo
/// and boundary backspace/delete still clearing pending state).
pub(super) fn finish_content_edit(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    finish_content_edit_with_message(app, out, None)
}

pub(super) fn finish_content_edit_with_message(
    app: &mut super::App,
    out: &mut dyn Write,
    message: Option<String>,
) -> io::Result<()> {
    completion::cancel(app);
    app.selection.clear();
    refresh_dirty(&mut app.file, &*app.buffer);
    if app.buffer.is_read_only() {
        app.message = Some("Large file is read-only in paged mode.".to_string());
    } else {
        app.pending_quit_confirm = false;
        app.pending_save_conflict = None;
        app.pending_reload = None;
        app.message = message;
    }
    app.reveal_cursor();
    app.render(out)
}

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
    if help::handle_key(app, out, key)? {
        return Ok(());
    }
    if recovery::handle_key(app, out, key)? {
        return Ok(());
    }
    if external_command::handle_key(app, out, key)? {
        return Ok(());
    }
    if repo_llm::handle_key(app, out, key)? {
        return Ok(());
    }
    if llm_request::handle_key(app, out, key)? {
        return Ok(());
    }
    if replace::handle_key(app, out, key)? {
        return Ok(());
    }
    if search::handle_active_key(app, out, key)? {
        return Ok(());
    }
    if command_prompt::handle_active_key(app, out, key)? {
        return Ok(());
    }
    if llm_preview::handle_key(app, out, key)? {
        return Ok(());
    }
    if llm_answer::handle_key(app, out, key)? {
        return Ok(());
    }
    if completion::handle_key(app, out, key)? {
        return Ok(());
    }
    if project_files::handle_key(app, out, key)? {
        return Ok(());
    }
    if lint::handle_key(app, out, key)? {
        return Ok(());
    }
    if view::is_preview(app) && view::handle_key(app, out, key)? {
        return Ok(());
    }
    let translated = app.keybindings.translate(key);
    if translated != key && completion::handle_key(app, out, translated)? {
        return Ok(());
    }
    let key = translated;
    if help::handle_key(app, out, key)? {
        return Ok(());
    }
    if view::handle_key(app, out, key)? {
        return Ok(());
    }
    if navigation::handle_key(app, out, key)? {
        return Ok(());
    }
    if selection::handle_shortcut(app, out, key)? {
        return Ok(());
    }
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
            handle_quit(app, out)?;
        }

        // Save As (Ctrl+Shift+S) opens a dedicated path prompt.
        KeyEvent {
            code: KeyCode::Char('s' | 'S'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL)
            && modifiers.contains(KeyModifiers::SHIFT) =>
        {
            command_prompt::open_save_as_prompt(app, out)?;
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

        KeyEvent {
            code: KeyCode::Char('f' | 'F'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL)
            && modifiers.contains(KeyModifiers::SHIFT) =>
        {
            replace::open_prompt(app, out, false)?;
        }

        KeyEvent {
            code: KeyCode::Char('f'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL) => {
            search::open_prompt(app, out)?;
        }

        KeyEvent {
            code: KeyCode::Char('g'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL) => {
            command_prompt::open_goto_prompt(app, out)?;
        }

        KeyEvent {
            code: KeyCode::Char('p' | 'P'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL)
            && modifiers.contains(KeyModifiers::SHIFT) =>
        {
            command_prompt::open_command_prompt(app, out)?;
        }

        KeyEvent {
            code: KeyCode::F(2),
            modifiers: KeyModifiers::NONE,
            ..
        } => {
            command_prompt::open_command_prompt(app, out)?;
        }

        KeyEvent {
            code: KeyCode::Char('o'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => command_prompt::open_file_prompt(app, out)?,

        KeyEvent {
            code: KeyCode::Char('n'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => command_prompt::execute_new(app, out)?,

        KeyEvent {
            code: KeyCode::Char('w'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => command_prompt::execute_close(app, out, false)?,

        KeyEvent {
            code: KeyCode::PageDown,
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::ALT) => {
            app.switch_buffer(buffers::BufferDirection::Next);
            app.reveal_cursor();
            app.render(out)?;
        }

        KeyEvent {
            code: KeyCode::PageUp,
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::ALT) => {
            app.switch_buffer(buffers::BufferDirection::Previous);
            app.reveal_cursor();
            app.render(out)?;
        }

        KeyEvent {
            code: KeyCode::PageDown,
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL) => {
            paging::handle_page_key(app, out, paging::PageDirection::Next)?;
        }

        KeyEvent {
            code: KeyCode::PageUp,
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL) => {
            paging::handle_page_key(app, out, paging::PageDirection::Previous)?;
        }

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

        // Enter produces KeyCode::Enter (not Char('\n')). Handle explicitly.
        // The Char \n/\r check below catches any that might arrive via paste
        // or other terminal paths.
        KeyEvent {
            code: KeyCode::Enter,
            ..
        } => {
            indentation::insert_newline(app, out)?;
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
            finish_content_edit(app, out)?;
        }
        KeyEvent {
            code: KeyCode::Char('z'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL)
            && modifiers.contains(KeyModifiers::SHIFT) =>
        {
            app.buffer.redo();
            finish_content_edit(app, out)?;
        }
        KeyEvent {
            code: KeyCode::Char('Z'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL)
            && modifiers.contains(KeyModifiers::SHIFT) =>
        {
            app.buffer.redo();
            finish_content_edit(app, out)?;
        }
        KeyEvent {
            code: KeyCode::Char('y'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL) => {
            app.buffer.redo();
            finish_content_edit(app, out)?;
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
                // Other Ctrl+letter combos ignored in Phase 0; still reveal/render (existing behavior)
            } else if c == '\n' || c == '\r' {
                indentation::insert_newline(app, out)?;
                return Ok(());
            } else if !c.is_control() {
                let ch = if modifiers.contains(KeyModifiers::SHIFT) && c.is_ascii_lowercase() {
                    c.to_ascii_uppercase()
                } else {
                    c
                };
                if app.selection.active().is_some() {
                    selection::replace_active(app, &ch.to_string())?;
                } else {
                    app.buffer.insert_char(ch);
                }
                finish_content_edit(app, out)?;
                return Ok(());
            }
            app.reveal_cursor();
            app.render(out)?;
        }

        KeyEvent {
            code: KeyCode::Backspace,
            ..
        } => {
            navigation::delete_grapheme(app, out, false)?;
        }

        KeyEvent {
            code: KeyCode::Delete,
            ..
        } => {
            navigation::delete_grapheme(app, out, true)?;
        }

        KeyEvent {
            code: KeyCode::Left,
            ..
        } => {
            app.selection.clear();
            navigation::move_grapheme(app, false)?;
            app.reveal_cursor();
            app.render(out)?;
        }

        KeyEvent {
            code: KeyCode::Right,
            ..
        } => {
            app.selection.clear();
            navigation::move_grapheme(app, true)?;
            app.reveal_cursor();
            app.render(out)?;
        }

        KeyEvent {
            code: KeyCode::Up, ..
        } => {
            app.selection.clear();
            app.buffer.move_up();
            navigation::snap_current_grapheme(app)?;
            app.reveal_cursor();
            app.render(out)?;
        }

        KeyEvent {
            code: KeyCode::Down,
            ..
        } => {
            app.selection.clear();
            app.buffer.move_down();
            navigation::snap_current_grapheme(app)?;
            app.reveal_cursor();
            app.render(out)?;
        }

        _ => {}
    }

    Ok(())
}

pub(crate) fn handle_paste(
    app: &mut super::App,
    out: &mut dyn Write,
    text: &str,
) -> io::Result<()> {
    completion::cancel(app);
    if help::handle_paste(app, out)? {
        return Ok(());
    }
    if replace::handle_paste(app, out, text)? {
        return Ok(());
    }
    if recovery::handle_paste(app, out)? {
        return Ok(());
    }
    if external_command::handle_paste(app, out)? {
        return Ok(());
    }
    if repo_llm::handle_paste(app, out)? {
        return Ok(());
    }
    if llm_request::handle_paste(app, out)? {
        return Ok(());
    }
    if llm_preview::handle_paste(app, out)? {
        return Ok(());
    }
    if llm_answer::handle_paste(app, out)? {
        return Ok(());
    }
    if project_files::handle_paste(app, out)? {
        return Ok(());
    }
    if lint::handle_paste(app, out)? {
        return Ok(());
    }
    if view::handle_paste(app, out)? {
        return Ok(());
    }
    selection::handle_external_paste(app, out, text)
}

pub(super) fn handle_quit(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let dirty_count = app.dirty_buffer_count();
    if dirty_count == 0 || app.pending_quit_confirm {
        app.should_quit = true;
        return Ok(());
    }
    app.pending_quit_confirm = true;
    app.message = Some(if dirty_count == 1 {
        "Unsaved changes. Press Ctrl+Q again to quit without saving, Ctrl+S to save.".to_string()
    } else {
        format!(
            "Unsaved changes in {dirty_count} buffers. Press Ctrl+Q again to quit without saving."
        )
    });
    app.render(out)
}
