//! Purpose: route normalized key and paste events into explicit App/editor actions.
//! Owns: key precedence, ordinary edit dispatch, and common post-edit cleanup.
//! Must not: decode raw terminal bytes, access buffer internals, render content, or network.
//! Invariants: scoped normalization precedes active surfaces; guarded editor actions win over
//!   text input; ordinary editor actions clear stale completed messages.

use std::io::{self, Write};

use crossterm::event::KeyEvent;

use crate::config::actions::{Action, Scope};

use super::file_state::{note_content_change, refresh_dirty};
use super::{
    buffers, command_prompt, completion, help, mobile, model_picker, navigation, overwrite, paging,
    reload, replace, save, search, selection, view,
};

mod editing;
mod surfaces;

mod scope;

#[cfg(test)]
mod dispatch_probe {
    use std::cell::Cell;

    use crate::config::actions::Action;

    thread_local! {
        static ENABLED: Cell<bool> = const { Cell::new(false) };
        static ACTION: Cell<Option<Action>> = const { Cell::new(None) };
    }

    pub(super) fn start() {
        ACTION.set(None);
        ENABLED.set(true);
    }

    pub(super) fn record(action: Action) -> bool {
        if !ENABLED.get() {
            return false;
        }
        ACTION.set(Some(action));
        true
    }

    pub(super) fn finish() -> Option<Action> {
        ENABLED.set(false);
        ACTION.take()
    }
}

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
    note_content_change(&mut app.file);
    completion::cancel(app);
    app.selection.clear();
    refresh_dirty(&mut app.file, &*app.buffer);
    app.clanker_changes
        .reconcile(app.buffer.edit_history_position());
    app.external_changes
        .reconcile(app.buffer.edit_history_position());
    if app.buffer.is_read_only() {
        app.message_warning("Large file is read-only in paged mode.");
    } else {
        command_prompt::clear_config_discard_confirmation(app);
        app.pending_quit_confirm = false;
        app.pending_save_conflict = None;
        app.pending_reload = None;
        app.message = message;
        app.message_role = crate::terminal::render::StatusRole::Info;
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
    if handle_bound_key(app, out, scope, key)? {
        return Ok(());
    }
    selection::end_cut_line_chain(app);
    handle_raw_key(app, out, key)
}

fn handle_bound_key(
    app: &mut super::App,
    out: &mut dyn Write,
    scope: Scope,
    key: KeyEvent,
) -> io::Result<bool> {
    if let Some(action) = app.keybindings.action_for_key(scope, key) {
        if action != Action::CutLine {
            selection::end_cut_line_chain(app);
        }
        dispatch_action(app, out, action)?;
        return Ok(true);
    }
    if app.keybindings.is_default_key(scope, key) {
        return Ok(true);
    }
    Ok(false)
}

fn handle_raw_key(app: &mut super::App, out: &mut dyn Write, key: KeyEvent) -> io::Result<()> {
    if surfaces::handle_raw_key(app, out, key)? {
        return Ok(());
    }
    editing::handle_key(app, out, key)
}

pub(super) fn prepare_editor_action(app: &mut super::App, action: Option<Action>) {
    let is_quit = matches!(action, Some(Action::Quit));
    let is_save = matches!(action, Some(Action::Save | Action::SaveAs))
        || (matches!(action, Some(Action::PromptSubmit)) && command_prompt::is_save_as_prompt(app));
    let is_reload = matches!(action, Some(Action::Reload));
    let keeps_confirmation = (is_quit
        && (app.pending_quit_confirm || command_prompt::config_discard_confirmation_pending(app)))
        || (is_save && app.pending_save_conflict.is_some())
        || (is_reload && app.pending_reload.is_some());
    if !is_quit {
        app.pending_quit_confirm = false;
        command_prompt::clear_config_discard_confirmation(app);
    }
    if !is_save {
        app.pending_save_conflict = None;
    }
    if !is_reload {
        app.pending_reload = None;
    }
    if !keeps_confirmation {
        app.message = None;
        app.message_role = crate::terminal::render::StatusRole::Info;
    }
}

pub(super) fn dispatch_action(
    app: &mut super::App,
    out: &mut dyn Write,
    action: Action,
) -> io::Result<()> {
    #[cfg(test)]
    if dispatch_probe::record(action) {
        return Ok(());
    }
    if action == Action::Interrupt {
        crate::terminal::request_interrupt();
        return Ok(());
    }
    prepare_editor_action(app, Some(action));
    match action {
        Action::Help => return help::toggle(app, out),
        Action::Quit => return handle_quit(app, out),
        _ => {}
    }
    let scope = active_scope(app);
    if scope != Scope::Editor {
        return surfaces::dispatch_action(app, out, scope, action);
    }
    if completion::dispatch_editor_action(app, out, action)? {
        return Ok(());
    }
    if editing::dispatch_action(app, out, action)?
        || navigation::dispatch_action(app, out, action)?
        || selection::dispatch_action(app, out, action)?
    {
        return Ok(());
    }
    match action {
        Action::Save => save::handle_save(app, out),
        Action::SaveAs => command_prompt::open_save_as_prompt(app, out),
        Action::Reload => reload::handle_reload_key(app, out),
        Action::Open => command_prompt::open_file_prompt(app, out),
        Action::New => command_prompt::execute_new(app, out),
        Action::Close => command_prompt::execute_close(app, out, false),
        Action::Search => search::open_prompt(app, out),
        Action::Replace => replace::open_prompt(app, out, false),
        Action::GotoLine => command_prompt::open_goto_prompt(app, out),
        Action::CommandPrompt => command_prompt::open_command_prompt(app, out),
        Action::Undo => {
            app.buffer.undo();
            finish_content_edit(app, out)
        }
        Action::Redo => {
            app.buffer.redo();
            finish_content_edit(app, out)
        }
        Action::ToggleOverwrite => overwrite::toggle(app, out),
        Action::Complete => completion::trigger(app, out),
        Action::PreviousBuffer => switch_buffer(app, out, buffers::BufferDirection::Previous),
        Action::NextBuffer => switch_buffer(app, out, buffers::BufferDirection::Next),
        Action::PreviousPage => paging::handle_page_key(app, out, paging::PageDirection::Previous),
        Action::NextPage => paging::handle_page_key(app, out, paging::PageDirection::Next),
        Action::ToggleExternalDiff
        | Action::MarkdownPreview
        | Action::LineNumbers
        | Action::Whitespace
        | Action::SoftWrap => view::dispatch_action(app, out, action).map(|_| ()),
        Action::RunClanker => super::hooks::before_inline_clanker(app, out),
        Action::ClearClankerChanges => super::inline_clanker::clear_changes(app, out),
        Action::SelectModel => model_picker::show(app, out),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn every_remapped_keyboard_action_reaches_semantic_dispatch() {
        let remapped = KeyEvent::new(KeyCode::F(12), KeyModifiers::ALT);
        let mut app = super::super::App::new(None).unwrap();
        let mut out = Vec::new();

        for descriptor in crate::config::actions::REGISTRY
            .iter()
            .filter(|descriptor| descriptor.input == crate::config::actions::InputKind::Keyboard)
        {
            let config = format!("[keybindings]\n{} = [\"alt+f12\"]\n", descriptor.name);
            app.keybindings = crate::config::keybindings::parse(&config)
                .unwrap_or_else(|error| panic!("{} remap must parse: {error}", descriptor.name));

            for scope in descriptor.scopes {
                dispatch_probe::start();
                assert!(
                    handle_bound_key(&mut app, &mut out, *scope, remapped).unwrap(),
                    "{} in {} was not consumed",
                    descriptor.name,
                    scope.name()
                );
                assert_eq!(
                    dispatch_probe::finish(),
                    Some(descriptor.action),
                    "{} in {}",
                    descriptor.name,
                    scope.name()
                );
            }
        }
    }

    #[test]
    fn unrelated_global_action_cancels_editor_confirmations() {
        let mut app = super::super::App::new(None).unwrap();
        let mut out = Vec::new();
        app.pending_quit_confirm = true;
        app.pending_save_conflict = Some(super::super::save::PendingSaveConflict {
            path: "save.txt".into(),
            status: crate::file::io::ExternalFileStatus::Modified,
            snapshot: None,
        });
        app.pending_reload = Some(super::super::reload::PendingReload {
            path: "reload.txt".into(),
            status: crate::file::io::ExternalFileStatus::Modified,
            snapshot: None,
        });

        dispatch_action(&mut app, &mut out, Action::Help).unwrap();

        assert!(!app.pending_quit_confirm);
        assert!(app.pending_save_conflict.is_none());
        assert!(app.pending_reload.is_none());
        assert!(super::super::help::is_viewing(&app));
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
                app.message_warning(
                    "Unsaved configuration. Press Ctrl+Q again to discard it, or Ctrl+S to save.",
                );
                return app.render(out);
            }
            command_prompt::ConfigCloseRequest::Close {
                return_target,
                discard,
            } => {
                if let Err(error) = app.close_active_buffer(discard) {
                    app.message_error(format!("Close error: {error}"));
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
    app.message_warning(if mobile::is_enabled(app) && dirty_count == 1 {
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
