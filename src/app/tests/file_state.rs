//! App file/dirty/save/quit/message tests (child submodule of app::tests).
//!
//! Purpose: this file must contain the tests for FileState, dirty tracking,
//! atomic save, quit confirmation guard, and minimal user messages.
//! Owns: the file/quit/save related #[test] functions moved here for module size.
//! Must not: runtime logic; included only under cfg(test).
//! Invariants: descendant of app::tests; uses `use super::super::*;` for private App
//!              seams (handle_key_with etc) and `use super::make_key;` for shared helper.
//! Phase: 2-i narrow cleanup (no behavior change; names stable within new module path).

use super::super::*;
use super::make_key;
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

// Phase 2-j: exact save-point dirty tracking tests (history token, no to_string compare on hot paths)

#[test]
fn app_file_state_new_and_open_initialize_saved_history_token_and_clean() {
    let app = App::new(None).unwrap();
    assert!(!app.file.dirty, "new starts clean");
    assert_eq!(
        app.file.saved_history_position,
        app.buffer.edit_history_position(),
        "saved token must match initial buffer position"
    );

    let app2 = App::new(Some("nonexistent_for_token_test.txt")).unwrap();
    assert!(!app2.file.dirty, "open missing starts clean");
    assert_eq!(
        app2.file.saved_history_position,
        app2.buffer.edit_history_position()
    );
}

#[test]
fn app_file_state_insert_then_save_then_undo_redo_exact_dirty() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_test_2j_token_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);

    let mut app = App::new(Some(&p)).unwrap();
    assert!(!app.file.dirty);
    let saved0 = app.file.saved_history_position;

    // insert makes dirty
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty, "insert makes dirty");
    assert!(app.buffer.edit_history_position() != saved0);

    // save marks clean at new token
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty, "save clears dirty");
    let saved_after = app.file.saved_history_position;
    assert!(saved_after != saved0);
    assert_eq!(saved_after, app.buffer.edit_history_position());

    // undo back to prior (away from saved) => dirty
    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty, "undo away from saved token makes dirty");
    assert_eq!(app.buffer.edit_history_position(), saved0);

    // redo back to saved => clean
    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty, "redo to saved token clears dirty");
    assert_eq!(
        app.file.saved_history_position,
        app.buffer.edit_history_position()
    );

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_undo_to_clean_then_redo_makes_dirty_again() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_test_2j_undo_clean_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('a'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);
    let clean_pos = app.file.saved_history_position;

    app.handle_key(make_key(KeyCode::Char('b'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);

    // undo the 'b' back exactly to saved
    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(
        !app.file.dirty,
        "undo to saved content must clear dirty exactly"
    );
    assert_eq!(app.buffer.edit_history_position(), clean_pos);

    // redo away
    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty, "redo away from saved must set dirty");

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_save_sets_new_clean_point_undo_redo_roundtrip() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_test_2j_save_point_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('1'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    let s1 = app.file.saved_history_position;

    app.handle_key(make_key(KeyCode::Char('2'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    let s2 = app.file.saved_history_position;
    assert!(s2 != s1, "second save must update saved token");
    assert!(!app.file.dirty);

    // undo to s1
    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    // redo to s2
    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);
    assert_eq!(app.file.saved_history_position, s2);

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_noop_undo_redo_on_clean_stays_clean() {
    let mut app = App::new(None).unwrap();
    assert!(!app.file.dirty);
    let p0 = app.buffer.edit_history_position();

    // no-op undo on clean
    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty, "no-op undo must not dirty a clean buffer");
    assert_eq!(app.buffer.edit_history_position(), p0);

    // no-op redo on clean
    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty, "no-op redo must not dirty a clean buffer");
    assert_eq!(app.buffer.edit_history_position(), p0);
}

#[test]
fn app_file_state_movement_render_resize_do_not_affect_dirty() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_test_2j_move_dirty_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);

    let mut app = App::new(Some(&p)).unwrap();
    // make dirty via content
    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    let pos_dirty = app.buffer.edit_history_position();

    // movement must not change dirty
    app.handle_key(make_key(KeyCode::Left, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Right, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Up, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    assert_eq!(app.buffer.edit_history_position(), pos_dirty);

    // render explicit must not
    let mut out = Vec::new();
    app.render(&mut out).unwrap();
    assert!(app.file.dirty);

    // resize
    let mut out2 = Vec::new();
    app.handle_resize(40, 12, &mut out2).unwrap();
    assert!(app.file.dirty);
    assert_eq!(app.buffer.edit_history_position(), pos_dirty);

    // now save to clean, movements still must not flip it
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);
    app.handle_key(make_key(KeyCode::Left, KeyModifiers::NONE))
        .unwrap();
    assert!(!app.file.dirty);

    let _ = std::fs::remove_file(&p);
}

// Phase 2-l file snapshot / external status tests (detection only; no watcher, no reload, no mutation)

#[test]
fn app_file_state_open_existing_stores_snapshot_and_clean() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2l_open_exist_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "abc\ndef\n").unwrap();

    let app = App::new(Some(&p)).unwrap();
    assert!(!app.file.dirty);
    assert!(app.file.path.is_some());
    match &app.file.disk_snapshot {
        Some(crate::file::io::FileSnapshot::Present { len, .. }) => {
            assert_eq!(*len, 8, "snapshot len must match file");
        }
        _ => panic!("expected Present snapshot for existing file"),
    }

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_open_missing_stores_absent_snapshot_and_clean() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2l_open_missing_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);

    let app = App::new(Some(&p)).unwrap();
    assert!(!app.file.dirty, "open missing must start clean");
    assert!(app.file.path.is_some());
    assert_eq!(
        app.file.disk_snapshot,
        Some(crate::file::io::FileSnapshot::Absent),
        "missing path must store explicit Absent snapshot"
    );

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_save_success_updates_snapshot_len() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2l_save_snap_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);

    let mut app = App::new(Some(&p)).unwrap();
    // type something
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE)).unwrap();
    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::NONE)).unwrap();
    assert!(app.file.dirty);

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL)).unwrap();
    assert!(!app.file.dirty);
    match &app.file.disk_snapshot {
        Some(crate::file::io::FileSnapshot::Present { len, .. }) => {
            assert_eq!(*len, 2, "snapshot after save must reflect written len");
        }
        _ => panic!("save success must set Present snapshot"),
    }

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_save_failure_leaves_snapshot_unchanged() {
    // Use a dir as target path to force atomic save error
    let mut bad = std::env::temp_dir();
    bad.push(format!("catomic_2l_bad_save_dir_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&bad);
    std::fs::create_dir_all(&bad).unwrap();
    assert!(bad.is_dir());

    let mut app = App::new(None).unwrap();
    // seed a path and a snapshot (as if previously saved cleanly)
    app.file.path = Some(bad.clone());
    // capture a fake snapshot for the dir (will be Absent or error but we set manually to a sentinel)
    app.file.disk_snapshot = Some(crate::file::io::FileSnapshot::Present { len: 42, mtime: None });
    app.file.dirty = true;

    let before = app.file.disk_snapshot.clone();

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL)).unwrap();

    assert!(app.file.dirty, "save error keeps dirty");
    assert_eq!(app.file.disk_snapshot, before, "snapshot must be unchanged on save failure");

    let _ = std::fs::remove_dir_all(&bad);
}

#[test]
fn app_file_state_external_append_reports_modified_no_mutation() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2l_ext_append_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "base").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    assert!(!app.file.dirty);
    let snap_before = app.file.disk_snapshot.clone();
    let dirty_before = app.file.dirty;
    let msg_before = app.message.clone();
    let pend_before = app.pending_quit_confirm;

    // external append (simulates other program)
    std::fs::write(&p, "baseEXT").unwrap(); // longer

    let status = app.external_file_status();
    assert_eq!(status, crate::file::io::ExternalFileStatus::Modified);

    // must not have mutated state
    assert_eq!(app.file.disk_snapshot, snap_before);
    assert_eq!(app.file.dirty, dirty_before);
    assert_eq!(app.message, msg_before);
    assert_eq!(app.pending_quit_confirm, pend_before);

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_external_delete_reports_deleted_no_mutation() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2l_ext_del_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "content").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL)).unwrap(); // ensure clean + snap
    assert!(!app.file.dirty);
    let before_dirty = app.file.dirty;
    let before_msg = app.message.clone();
    let before_pend = app.pending_quit_confirm;

    // external delete
    let _ = std::fs::remove_file(&p);

    let status = app.external_file_status();
    assert_eq!(status, crate::file::io::ExternalFileStatus::Deleted);

    assert_eq!(app.file.dirty, before_dirty);
    assert_eq!(app.message, before_msg);
    assert_eq!(app.pending_quit_confirm, before_pend);

    // cleanup
    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_no_path_reports_nopath() {
    let app = App::new(None).unwrap();
    assert!(app.file.path.is_none());
    assert_eq!(
        app.external_file_status(),
        crate::file::io::ExternalFileStatus::NoPath
    );
}
