//! Focused deterministic tests for watcher signal application and check seam (Phase 2-aa/2-ab).
//!
//! Purpose: exercise apply_file_watch_signal + check_file_watcher_once + (future) runtime helper seam.
//! Owns: signal hint behavior tests (arms like Ctrl+R, no auto reload, no mutation of content/dirty/snap).
//! Must not: rely on live OS notify delivery; introduce flakiness; test reload content paths; change manual Ctrl+R semantics.
//! Invariants: signals are hints only; always fresh observe + apply_check_observation; same arming as first Ctrl+R;
//!              direct apply tests and check-on-no-signal tests remain stable.
//! Phase: 2-aa (seams) + 2-ab (runtime wiring + split cleanup).

use super::super::super::*;
use super::super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};

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

// Phase 2-ab: deterministic runtime seam tests for the check-and-render helper.
// These exercise the loop integration point without live notify or event injection.
// A real queued Changed/Deleted from the OS would cause the helper to return true,
// render, and arm (via the existing apply path); that delivery path is integration-only
// (see TODO.md: no live OS notify integration tests in default/CI suite).
// We cover the stable no-signal cases + assert no spurious render.

#[test]
fn check_file_watcher_once_and_render_no_watcher_returns_false_writes_nothing() {
    let mut app = App::new(None).unwrap();
    assert!(app.file_watcher.is_none());

    let mut out: Vec<u8> = Vec::new();
    let had =
        crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap();
    assert!(!had, "no watcher => no signal handled");
    assert!(
        out.is_empty(),
        "must not call render when no signal (output len={})",
        out.len()
    );
}

#[test]
fn check_file_watcher_once_and_render_watcher_no_signal_returns_false_writes_nothing() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2ab_render_nosig_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "SEAMDATA").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    assert!(
        app.file_watcher.is_some(),
        "parent watcher present for test"
    );

    let mut out: Vec<u8> = Vec::new();
    let had =
        crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap();
    assert!(
        !had,
        "fresh watcher with no queued event must not report handled"
    );
    assert!(
        out.is_empty(),
        "must not render on no-signal (would emit buffer bytes)"
    );

    let _ = std::fs::remove_file(&p);
}

// Note: a deterministic "signal queued => helper true + arms + renders" test would
// require either (a) live FS notify wait (flaky, forbidden for default tests) or
// (b) a test seam to inject into the watcher's mpsc (would expand watcher surface).
// Per instructions, skipped; runtime delivery covered by the apply tests + manual later.
