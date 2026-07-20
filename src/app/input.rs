//! Purpose: route normalized key and paste events into explicit App/editor actions.
//! Owns: key precedence, ordinary edit dispatch, and common post-edit cleanup.
//! Must not: decode raw terminal bytes, access buffer internals, render content, or network.
//! Invariants: scoped normalization precedes active surfaces; guarded editor actions win over
//!   text input; one user edit clears stale confirmations.
//! Phase: 3-d keyboard selection, with bounded post-beta routing cleanup.

use std::io::{self, Write};

use crossterm::event::KeyEvent;

use crate::help_catalog::{self, EditorAction};

use super::file_state::refresh_dirty;
use super::{
    autocomplete, buffers, command_prompt, completion, help, mobile, model_picker, overwrite,
    paging, reload, replace, save, search, selection, view,
};

mod editing;
mod shortcuts;
mod surfaces;

mod scope;

pub(super) fn active_scope(app: &super::App) -> crate::config::actions::Scope {
    scope::active(app)
}

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
    autocomplete::note_content_edit(app);
    completion::cancel(app);
    app.selection.clear();
    refresh_dirty(&mut app.file, &*app.buffer);
    app.clanker_changes
        .reconcile(app.buffer.edit_history_position());
    if app.buffer.is_read_only() {
        app.message = Some("Large file is read-only in paged mode.".to_string());
    } else {
        command_prompt::clear_config_discard_confirmation(app);
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
    if mobile::handle_key(app, out, key)? {
        selection::end_cut_line_chain(app);
        return Ok(());
    }
    let scope = scope::active(app);
    let Some(key) = app.keybindings.translate(scope, key) else {
        return Ok(());
    };
    if shortcuts::is_interrupt_key(key) {
        crate::terminal::request_interrupt();
        return Ok(());
    }
    if scope != crate::config::actions::Scope::Editor || !selection::is_cut_line_key(key) {
        selection::end_cut_line_chain(app);
    }
    handle_normalized_key(app, out, key)
}

/// Dispatch a canonical key without consulting user hardware bindings.
/// Mobile actions use this after explicit hit testing so unbinding a keyboard
/// chord cannot make its corresponding touch action unreachable.
pub(super) fn handle_normalized_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<()> {
    if surfaces::handle_raw_key(app, out, key)? {
        return Ok(());
    }
    if shortcuts::handle_inline_clanker_key(app, out, key)? {
        return Ok(());
    }
    if surfaces::handle_translated_key(app, out, key)? {
        return Ok(());
    }
    if shortcuts::handle_key(app, out, key)? {
        return Ok(());
    }
    editing::handle_key(app, out, key)
}

pub(super) fn dispatch_editor_action(
    app: &mut super::App,
    out: &mut dyn Write,
    action: EditorAction,
) -> io::Result<()> {
    match action {
        EditorAction::Help => help::show(app, out),
        EditorAction::Quit => handle_quit(app, out),
        EditorAction::Save => save::handle_save(app, out),
        EditorAction::SaveAs => command_prompt::open_save_as_prompt(app, out),
        EditorAction::Reload => reload::handle_reload_key(app, out),
        EditorAction::Open => command_prompt::open_file_prompt(app, out),
        EditorAction::New => command_prompt::execute_new(app, out),
        EditorAction::Close => command_prompt::execute_close(app, out, false),
        EditorAction::Search => search::open_prompt(app, out),
        EditorAction::Replace => replace::open_prompt(app, out, false),
        EditorAction::GotoLine => command_prompt::open_goto_prompt(app, out),
        EditorAction::CommandPrompt => command_prompt::open_command_prompt(app, out),
        EditorAction::Undo => {
            app.buffer.undo();
            finish_content_edit(app, out)
        }
        EditorAction::Redo => {
            app.buffer.redo();
            finish_content_edit(app, out)
        }
        EditorAction::ToggleOverwrite => overwrite::toggle(app, out),
        EditorAction::Complete => {
            completion::handle_key(app, out, help_catalog::canonical_key(action)).map(|_| ())
        }
        EditorAction::PreviousBuffer => switch_buffer(app, out, buffers::BufferDirection::Previous),
        EditorAction::NextBuffer => switch_buffer(app, out, buffers::BufferDirection::Next),
        EditorAction::PreviousPage => {
            paging::handle_page_key(app, out, paging::PageDirection::Previous)
        }
        EditorAction::NextPage => paging::handle_page_key(app, out, paging::PageDirection::Next),
        EditorAction::MarkdownPreview
        | EditorAction::LineNumbers
        | EditorAction::Whitespace
        | EditorAction::SoftWrap => {
            view::handle_key(app, out, help_catalog::canonical_key(action)).map(|_| ())
        }
        EditorAction::SelectModel => model_picker::show(app, out),
    }
}

fn switch_buffer(
    app: &mut super::App,
    out: &mut dyn Write,
    direction: buffers::BufferDirection,
) -> io::Result<()> {
    app.switch_buffer(direction);
    app.reveal_cursor();
    app.render(out)
}

pub(crate) fn handle_paste(
    app: &mut super::App,
    out: &mut dyn Write,
    text: &str,
) -> io::Result<()> {
    selection::end_cut_line_chain(app);
    if mobile::handle_paste(app, out)? {
        return Ok(());
    }
    if autocomplete::handle_paste(app, out)? {
        return Ok(());
    }
    completion::cancel(app);
    if surfaces::handle_paste(app, out, text)? {
        return Ok(());
    }
    selection::handle_external_paste(app, out, text)
}

pub(super) fn handle_quit(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if let Some(request) = command_prompt::request_config_close(app) {
        match request {
            command_prompt::ConfigCloseRequest::WarnDirty => {
                app.message = Some(
                    "Unsaved configuration. Press Ctrl+Q again to discard it, or Ctrl+S to save."
                        .to_string(),
                );
                return app.render(out);
            }
            command_prompt::ConfigCloseRequest::Close {
                return_target,
                discard,
            } => {
                if let Err(error) = app.close_active_buffer(discard) {
                    app.message = Some(format!("Close error: {error}"));
                    return app.render(out);
                }
                app.message = None;
                for _ in 0..app.buffer_count() {
                    if app.active_buffer_index == return_target
                        || !app.switch_buffer(buffers::BufferDirection::Previous)
                    {
                        break;
                    }
                }
                app.message = None;
                app.reveal_cursor();
                return app.render(out);
            }
        }
    }

    let dirty_count = app.dirty_buffer_count();
    if dirty_count == 0 || app.pending_quit_confirm {
        app.should_quit = true;
        return Ok(());
    }
    app.pending_quit_confirm = true;
    app.message = Some(if mobile::is_enabled(app) && dirty_count == 1 {
        "Unsaved changes. Tap Menu > Quit again to discard, or tap Save.".to_string()
    } else if mobile::is_enabled(app) {
        format!("Unsaved changes in {dirty_count} buffers. Tap Menu > Quit again to discard.")
    } else if dirty_count == 1 {
        "Unsaved changes. Press Ctrl+Q again to quit without saving, Ctrl+S to save.".to_string()
    } else {
        format!(
            "Unsaved changes in {dirty_count} buffers. Press Ctrl+Q again to quit without saving."
        )
    });
    app.render(out)
}
