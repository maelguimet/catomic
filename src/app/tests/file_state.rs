//! App file/dirty/save/quit/message tests (child submodule of app::tests; hub for split).
//!
//! Purpose: hub for file_state tests after 2-o split. Declares submodules for
//! focused groups (dirty, snapshot, save_conflict). Owns remaining (e.g. pure quit guards).
//! Must not: runtime logic; included only under cfg(test).
//! Invariants: all original test names preserved exactly; submodules use super::super::*;
//!              no behavior change.
//! Phase: 2-o narrow cleanup.

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

// Phase 2-b quit guard + message tests (via simulated keys; no real terminal)

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
    // make dirty
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    assert!(!app.pending_quit_confirm);
    assert!(app.message.is_none());

    // first Ctrl+Q: no quit, sets pending + msg
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.should_quit, "first dirty Q does not quit");
    assert!(app.pending_quit_confirm);
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("Unsaved changes") && msg.contains("Ctrl+Q again"),
        "message should warn: got {:?}",
        app.message
    );

    // second Ctrl+Q: quits
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.should_quit, "second dirty Q quits");
}

#[test]
fn app_dirty_ctrl_q_first_renders_warning_immediately() {
    // Regression for invisible warning: first dirty Ctrl+Q must emit render
    // containing the message on bottom row (via the writer seam).
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

// Phase 2-al input hygiene: explicit coverage that unrelated editor actions cancel
// pending confirmations while the matching action remains armed.

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

    // Any content mutation (printable char) must clear via the shared path.
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
    // Make dirty so Q will arm pending instead of immediate quit.
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_quit_confirm);
    assert!(app.message.is_some());

    // Undo (there is history) clears via finish_content_edit.
    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.pending_quit_confirm);
    assert!(app.message.is_none());

    // Re-arm pending via Q (still dirty from the 'x' that was undone? wait: after undo we are at saved, so make dirty again).
    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_quit_confirm);
    // Redo no history in this state (or after undo boundary) still exercises the clear path.
    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.pending_quit_confirm);
}

#[test]
fn movement_cancels_save_conflict_and_reload_pending() {
    let mut app = App::new(None).unwrap();
    // Directly arm both to exercise the shared unrelated-action cancellation path.
    app.pending_save_conflict = Some(super::super::save::PendingSaveConflict {
        path: std::path::PathBuf::from("x"),
        status: crate::file::io::ExternalFileStatus::Modified,
        snapshot: None,
    });
    app.pending_reload = Some(super::super::reload::PendingReload {
        path: std::path::PathBuf::from("x"),
        status: crate::file::io::ExternalFileStatus::Modified,
        snapshot: None,
    });
    app.message_info("armed");

    app.handle_key(make_key(KeyCode::Left, KeyModifiers::NONE))
        .unwrap();
    assert!(
        app.pending_save_conflict.is_none(),
        "movement must cancel save conflict pending"
    );
    assert!(
        app.pending_reload.is_none(),
        "movement must cancel reload pending"
    );
    assert!(app.message.is_none());
}

// (hub ends cleanly)
