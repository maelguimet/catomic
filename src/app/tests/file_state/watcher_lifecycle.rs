//! Focused App FileWatcher lifecycle tests (Phase 2-z).
//!
//! Purpose: verify App owns a gated FileWatcher after new(path) and after
//! successful first-save path creation; failure paths leave None; no signals
//! are ever received here.
//! Owns: pure ctor and post-assign lifecycle assertions using temp paths.
//! Must not: consume try_recv, drive any reload, add live OS event waits,
//!   mutate behavior of save conflict, use set_current_dir, or touch Project/LLM.
//! Invariants: watcher presence only; construction non-fatal; uses existing
//!   Plain caps; tests the helper directly for the "after save assign" case
//!   to avoid hardcoded "untitled.txt" cwd writes in potentially parallel runs.
//! Phase: 2-z narrow pass (lifecycle preparation only).

use super::super::super::*;
use super::super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};

#[test]
fn app_new_no_path_has_no_file_watcher() {
    let app = App::new(None).unwrap();
    assert!(app.file.path.is_none());
    assert!(
        app.file_watcher.is_none(),
        "App::new(None) must start with no watcher"
    );
}

#[test]
fn app_new_with_existing_temp_file_gets_watcher_under_plain_caps() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2z_exist_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "hello 2z").unwrap();

    let app = App::new(Some(&p)).unwrap();
    assert!(app.file.path.is_some());
    // Parent exists; Plain has file_watch=true; should get a watcher (parent dir).
    assert!(
        app.file_watcher.is_some(),
        "watcher must be Some for file in existing parent under Plain caps"
    );

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_new_missing_file_in_existing_parent_still_gets_parent_watcher() {
    // Target does not exist, but parent (temp) is watchable -> watcher Some.
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2z_missing_in_parent_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    // do not create the file

    let app = App::new(Some(&p)).unwrap();
    assert!(app.file.path.is_some());
    assert!(
        app.file_watcher.is_some(),
        "parent watcher must succeed for missing target when parent exists"
    );

    // no file to clean
}

#[test]
fn app_new_with_nonexistent_parent_succeeds_but_watcher_none() {
    // App::new must succeed even if we cannot watch (nonexistent parent).
    // Watcher must be None; editing must not be prevented.
    let mut bad = std::env::temp_dir();
    bad.push("catomic_2z_no_parent_$$_dir");
    bad.push(format!("sub_{}/target.txt", std::process::id()));
    // ensure the deep parent chain does not exist
    let _ = std::fs::remove_dir_all(bad.parent().unwrap());

    let app = App::new(Some(bad.to_str().unwrap())).unwrap();
    assert!(
        app.file.path.is_some(),
        "path must be remembered even if unwatchable"
    );
    assert!(
        app.file_watcher.is_none(),
        "watcher must be None when parent dir does not exist (ctor non-fatal)"
    );
}

#[test]
fn save_error_does_not_attach_or_replace_watcher() {
    // Use the save-failure pattern (target is a directory) to force atomic write error.
    // After failed save, watcher must remain None (from new(None) + no path assign happened).
    let mut bad = std::env::temp_dir();
    bad.push(format!("catomic_2z_bad_save_dir_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&bad);
    std::fs::create_dir_all(&bad).unwrap();
    assert!(bad.is_dir());

    let mut app = App::new(None).unwrap();
    assert!(app.file_watcher.is_none());
    // seed a path that will fail on atomic save (dir)
    app.file.path = Some(bad.clone());
    app.file.dirty = true;

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(app.file.dirty, "error path keeps dirty");
    // No successful path creation occurred; watcher must not have become Some.
    assert!(
        app.file_watcher.is_none(),
        "save error must not attach a watcher"
    );

    let _ = std::fs::remove_dir_all(&bad);
}

#[test]
fn refresh_after_path_assign_on_watchable_parent_sets_watcher() {
    // Directly exercise the post-assign refresh (what successful untitled save does)
    // without relying on the literal "untitled.txt" cwd write. This is safe for
    // parallel test runs and exercises the helper + watcher ctor path.
    let mut app = App::new(None).unwrap();
    assert!(app.file.path.is_none());
    assert!(app.file_watcher.is_none());

    // Simulate the exact state after a successful first-save path assignment:
    // a concrete path in a real, watchable parent (temp dir exists).
    let mut target = std::env::temp_dir();
    target.push(format!(
        "catomic_2z_helper_refresh_{}.txt",
        std::process::id()
    ));
    // target file need not exist for parent watcher to succeed.
    let _ = std::fs::remove_file(&target);
    app.file.path = Some(target.clone());

    // This is exactly what save.rs does after assign on success.
    crate::app::watch::refresh_file_watcher(&mut app);

    assert!(
        app.file_watcher.is_some(),
        "refresh after assigning watchable path must produce a watcher"
    );

    let _ = std::fs::remove_file(&target);
}

// Phase 2-aa: apply_file_watch_signal deterministic tests (signals are hints only).
// Always use fresh observe_external_file + apply_check_observation (same as Ctrl+R).
// Never trust signal variant for content action; no reload, no dirty/snapshot changes.

#[test]
fn apply_file_watch_signal_changed_on_unchanged_disk_sets_unchanged_message() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2aa_sig_unch_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap(); // ensure clean snapshot
    assert!(!app.file.dirty);

    // Simulate a Changed signal (e.g. from watcher)
    let sig = crate::file::watcher::FileWatchSignal::Changed;
    crate::app::watch::apply_file_watch_signal(&mut app, sig);

    assert_eq!(app.message.as_deref(), Some("File unchanged on disk."));
    assert!(app.pending_reload.is_none());
    assert_eq!(app.buffer.to_string(), "BASE");
    assert!(!app.file.dirty);

    let _ = std::fs::remove_file(&p);
}

#[test]
fn apply_file_watch_signal_changed_external_modified_arms_like_first_ctrl_r() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2aa_sig_mod_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "ORIG").unwrap();

    let mut app = App::new(Some(&p)).unwrap();

    // External change
    std::fs::write(&p, "ORIGEXT").unwrap();

    let sig = crate::file::watcher::FileWatchSignal::Changed;
    crate::app::watch::apply_file_watch_signal(&mut app, sig);

    // Same arming as first Ctrl+R on Modified (clean case)
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("changed on disk"));
    assert!(app.pending_reload.is_some());
    assert_eq!(app.buffer.to_string(), "ORIG"); // no reload
    assert!(!app.file.dirty);

    let _ = std::fs::remove_file(&p);
}

#[test]
fn apply_file_watch_signal_changed_dirty_external_arms_with_discard_warning() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2aa_sig_mod_dirty_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap(); // local dirty
    assert!(app.file.dirty);

    // External change
    std::fs::write(&p, "BASEEXT").unwrap();

    let sig = crate::file::watcher::FileWatchSignal::Changed;
    crate::app::watch::apply_file_watch_signal(&mut app, sig);

    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("changed on disk") && msg.contains("discard"),
        "dirty external Modified must warn about discard: got {:?}",
        app.message
    );
    assert!(app.pending_reload.is_some());
    assert!(app.file.dirty, "must not clear dirty");
    assert_eq!(app.buffer.to_string(), "xBASE"); // local edit preserved

    let _ = std::fs::remove_file(&p);
}

#[test]
fn apply_file_watch_signal_deleted_arms_like_first_ctrl_r() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2aa_sig_del_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "TODEL").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);

    let _ = std::fs::remove_file(&p); // external delete

    let sig = crate::file::watcher::FileWatchSignal::Deleted;
    crate::app::watch::apply_file_watch_signal(&mut app, sig);

    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("deleted on disk"),
        "Deleted signal must arm like Ctrl+R: got {:?}",
        app.message
    );
    assert!(app.pending_reload.is_some());
    assert_eq!(app.buffer.to_string(), "TODEL");
    assert!(!app.file.dirty);

    // re-create for cleanup
    std::fs::write(&p, "TODEL").unwrap();
    let _ = std::fs::remove_file(&p);
}

#[test]
fn apply_file_watch_signal_error_sets_message_only() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2aa_sig_err_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "EBASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    let before_dirty = app.file.dirty;
    let before_buf = app.buffer.to_string();
    let before_snap = app.file.disk_snapshot.clone();

    let sig = crate::file::watcher::FileWatchSignal::Error("boom".to_string());
    crate::app::watch::apply_file_watch_signal(&mut app, sig);

    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.starts_with("File watcher error:"),
        "error message must start with prefix: got {:?}",
        app.message
    );
    assert_eq!(app.buffer.to_string(), before_buf);
    assert_eq!(app.file.dirty, before_dirty);
    assert_eq!(app.file.disk_snapshot, before_snap);
    // pending state left as-is (no concrete reason to clear)

    let _ = std::fs::remove_file(&p);
}

// Phase 2-aa: check_file_watcher_once (non-runtime seam) tests.
// Only tests no-watcher and "watcher present but no queued signal" (stable, no live wait).
// Real event delivery would require OS notify which is out of scope for deterministic tests.

#[test]
fn check_file_watcher_once_no_watcher_returns_false_no_mutation() {
    let mut app = App::new(None).unwrap();
    assert!(app.file_watcher.is_none());

    let before_msg = app.message.clone();
    let before_pend = app.pending_reload.clone();
    let before_dirty = app.file.dirty;

    let had = crate::app::watch::check_file_watcher_once(&mut app);
    assert!(!had);
    assert_eq!(app.message, before_msg);
    assert_eq!(app.pending_reload, before_pend);
    assert_eq!(app.file.dirty, before_dirty);
}

#[test]
fn check_file_watcher_once_with_watcher_no_signal_returns_false_no_mutation() {
    // Construct App with a real temp file -> watcher Some (parent exists).
    // Immediately after new there should be no pending notify event in the mpsc.
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2aa_check_nosig_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "DATA").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    assert!(
        app.file_watcher.is_some(),
        "expect watcher for existing parent"
    );

    let before_msg = app.message.clone();
    let before_pend = app.pending_reload.clone();

    let had = crate::app::watch::check_file_watcher_once(&mut app);
    assert!(
        !had,
        "no queued signal expected immediately after construct"
    );
    assert_eq!(app.message, before_msg);
    assert_eq!(app.pending_reload, before_pend);

    let _ = std::fs::remove_file(&p);
}
