//! App file/dirty/save/quit/message tests (child submodule of app::tests; hub for split).
//!
//! Purpose: hub for file_state tests after 2-o split. Declares submodules for
//! focused groups (dirty, snapshot, save_conflict). Owns remaining (e.g. pure quit guards).
//! Must not: runtime logic; included only under cfg(test).
//! Invariants: all original test names preserved exactly; submodules use super::super::*;
//!              no behavior change.

use super::super::*;
use super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};

mod dirty;
mod file_size;
mod file_size_open;
mod large_editable;
mod metadata_collision;
mod save_conflict;
mod snapshot;
mod text_format;
mod watcher_acceptance;
mod watcher_lifecycle;
mod watcher_live;
mod watcher_pending;
mod watcher_runtime;
mod watcher_signal;

#[test]
fn app_quit_clean_immediately() {
    let mut app = App::new(None).unwrap();
    assert!(!app.file.dirty);
    assert!(!app.should_quit);
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.should_quit, "clean Ctrl+Q quits immediately");
}

#[test]
fn app_quit_dirty_first_sets_pending_and_message_second_quits() {
    let mut app = App::new(None).unwrap();
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    assert!(!app.pending_quit_confirm);
    assert!(app.message.is_none());

    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.should_quit, "first dirty Q does not quit");
    assert!(app.pending_quit_confirm);
    assert_eq!(
        app.message_role,
        crate::terminal::render::StatusRole::Warning
    );
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("Unsaved changes") && msg.contains("Ctrl+Q again"),
        "message should warn: got {:?}",
        app.message
    );

    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.should_quit, "second dirty Q quits");
}

#[test]
fn app_dirty_ctrl_q_first_renders_warning_immediately() {
    let mut app = App::new(None).unwrap();
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    assert!(app.message.is_none());

    let mut out: Vec<u8> = Vec::new();
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('q'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert!(!app.should_quit, "first dirty Q does not quit");
    assert!(app.pending_quit_confirm);
    let rendered = String::from_utf8_lossy(&out);
    assert!(
        rendered.contains("Unsaved changes") && rendered.contains("Ctrl+Q again"),
        "warning message text must appear in render output"
    );
    assert!(
        rendered.contains("\x1b[2K"),
        "render must clear the complete bottom row before the warning"
    );
}

#[test]
fn movement_cancels_pending_quit_and_message() {
    let mut app = App::new(None).unwrap();
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_quit_confirm);
    assert!(app.message.is_some());

    app.handle_key(make_key(KeyCode::Left, KeyModifiers::NONE))
        .unwrap();
    assert!(!app.pending_quit_confirm);
    assert!(app.message.is_none());
}

#[test]
fn content_edit_clears_pending_quit_and_message() {
    let mut app = App::new(None).unwrap();
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_quit_confirm);
    assert!(app.message.is_some());

    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::NONE))
        .unwrap();
    assert!(
        !app.pending_quit_confirm,
        "content edit must clear pending quit"
    );
    assert!(app.message.is_none(), "content edit must clear message");
}

#[test]
fn undo_redo_clear_pending_and_message_even_on_noop() {
    let mut app = App::new(None).unwrap();
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_quit_confirm);
    assert!(app.message.is_some());

    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.pending_quit_confirm);
    assert!(app.message.is_none());

    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_quit_confirm);
    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.pending_quit_confirm);
}

#[test]
fn movement_cancels_save_conflict_and_downgrades_reload_confirmation() {
    let mut app = App::new(None).unwrap();
    app.pending_save_conflict = Some(super::super::save::PendingSaveConflict {
        path: std::path::PathBuf::from("x"),
        status: crate::file::io::ExternalFileStatus::Modified,
        snapshot: None,
        is_command_prompt_save_as: false,
    });
    app.pending_reload = Some(super::super::reload::PendingReload {
        path: std::path::PathBuf::from("x"),
        status: crate::file::io::ExternalFileStatus::Modified,
        snapshot: None,
        is_explicitly_armed: true,
    });
    app.message_warning("armed");

    app.handle_key(make_key(KeyCode::Left, KeyModifiers::NONE))
        .unwrap();
    assert!(
        app.pending_save_conflict.is_none(),
        "movement must cancel save conflict pending"
    );
    assert!(
        app.pending_reload
            .as_ref()
            .is_some_and(|pending| !pending.is_explicitly_armed),
        "movement must retain only the passive reload observation"
    );
    assert!(app.message.is_none());
}
