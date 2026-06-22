//! App tests (child module split out of app.rs).
//!
//! Purpose: this file must contain the tests for App high-level state, key handling,
//! resize/reveal/scroll invariants, dirty tracking, quit guard, and render seams.
//! Owns: all cfg(test) tests and the make_key helper for simulated input.
//! Must not: contain any runtime logic or be included outside test builds.
//! Invariants: loaded only under #[cfg(test)] via `mod tests;` in app.rs;
//!              uses `use super::*;` to access private App methods (e.g. handle_key_with).
//! Phase: 2-g cleanup (no behavior change).

mod viewport;

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[test]
fn app_file_state_new_starts_clean() {
    let app = App::new(None).unwrap();
    assert!(!app.file.dirty, "new app without path starts clean");
    assert!(app.file.path.is_none());
    // screen field added in 2-c; verify default here too (no behavior change)
    assert_eq!(app.screen.height, 24);
    assert_eq!(app.screen.scroll_top, 0);

    let app2 = App::new(Some("existing.txt")).unwrap();
    assert!(!app2.file.dirty, "open (even missing file) starts clean");
    assert_eq!(
        app2.file.path.as_deref(),
        Some(std::path::Path::new("existing.txt"))
    );
}

#[test]
fn app_dirty_lifecycle_via_keys() {
    // Use explicit temp path for the test so we NEVER write bare "untitled.txt"
    // into the repo cwd. App::new with a path (even non-existing) starts clean
    // and save will target that path instead of defaulting.
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_test_dirty_lifecycle_{}_{}.txt",
        std::process::id(),
        "lifecycle"
    ));
    let test_path = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&test_path); // ensure clean start

    let mut app = App::new(Some(&test_path)).unwrap();
    assert!(!app.file.dirty);
    assert_eq!(
        app.file.path.as_deref(),
        Some(std::path::Path::new(&test_path))
    );

    // char insert marks dirty
    app.handle_key(KeyEvent {
        code: KeyCode::Char('a'),
        modifiers: KeyModifiers::NONE,
        kind: crossterm::event::KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    })
    .unwrap();
    assert!(app.file.dirty, "edit marks dirty");

    // save (via atomic) clears dirty; uses explicit path (no untitled.txt)
    app.handle_key(KeyEvent {
        code: KeyCode::Char('s'),
        modifiers: KeyModifiers::CONTROL,
        kind: crossterm::event::KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    })
    .unwrap();
    assert!(!app.file.dirty, "successful save marks clean");
    assert!(app.file.path.is_some());

    // edit after save marks dirty again
    app.handle_key(KeyEvent {
        code: KeyCode::Char('b'),
        modifiers: KeyModifiers::NONE,
        kind: crossterm::event::KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    })
    .unwrap();
    assert!(app.file.dirty, "post-save edit marks dirty again");

    // Clean up ONLY the temp path created/used by this test.
    let _ = std::fs::remove_file(&test_path);
}

// Phase 2-b quit guard + message tests (via simulated keys; no real terminal)

pub(super) fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: crossterm::event::KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    }
}

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
        rendered.contains("\x1b[K"),
        "render must clear bottom row with \\x1b[K even for message"
    );
}

#[test]
fn app_ctrl_s_after_dirty_clears_dirty_and_pending() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_test_save_clears_pending_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);

    // trigger quit warn
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_quit_confirm);

    // Ctrl+S: success clears dirty + pending + msg
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);
    assert!(!app.pending_quit_confirm);
    assert!(app.message.is_none());

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_save_error_keeps_dirty_and_sets_error_message() {
    // Use a dedicated subdir under temp (never bare temp_dir or root sibling)
    // so that path points to a directory -> atomic_write fails as intended.
    let mut bad = std::env::temp_dir();
    bad.push(format!("catomic_bad_save_dir_{}", std::process::id()));
    // ensure clean and is a dir
    let _ = std::fs::remove_dir_all(&bad);
    std::fs::create_dir_all(&bad).expect("create dedicated bad dir");
    assert!(bad.is_dir());

    let mut app = App::new(None).unwrap();
    app.file.path = Some(bad.clone());
    app.file.dirty = true;
    app.message = None;

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(app.file.dirty, "save error must keep dirty=true");
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("Save error") || msg.contains("error"),
        "save error should set message, got: {:?}",
        app.message
    );

    // cleanup dedicated dir only
    let _ = std::fs::remove_dir_all(&bad);
}

#[test]
fn app_edit_after_quit_warning_clears_pending() {
    let mut app = App::new(None).unwrap();
    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);

    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_quit_confirm);
    assert!(app.message.is_some());

    // content-mutating edit clears BOTH pending and message (movements do not)
    app.handle_key(make_key(KeyCode::Char('!'), KeyModifiers::NONE))
        .unwrap();
    assert!(
        !app.pending_quit_confirm,
        "edit after warning clears pending"
    );
    assert!(
        app.message.is_none(),
        "edit after warning also clears stale message"
    );
}
