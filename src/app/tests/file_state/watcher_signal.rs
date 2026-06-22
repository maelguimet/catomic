//! Focused deterministic tests for watcher signal application and check seam.
//!
//! Purpose: exercise apply_file_watch_signal + check_file_watcher_once directly
//!   (no render). These cover hint behavior and the suppression of Unchanged/NoPath
//!   from the watcher path (ignored to avoid self-save noise).
//! Owns: direct apply + simple check seam tests.
//! Must not: rely on live OS notify delivery; test reload content paths; change
//!   manual Ctrl+R (reload::apply_check_observation) semantics.
//! Invariants: signals are hints only; watcher Unchanged/NoPath observations are
//!   ignored (no message, no arm); direct apply + no-signal check tests stable.
//! Phase: 2-ac.

use super::super::super::*;
use super::super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};

// Phase 2-aa: apply_file_watch_signal deterministic tests (signals are hints only).
// Always use fresh observe_external_file + apply_check_observation (same as Ctrl+R).
// Never trust signal variant for content action; no reload, no dirty/snapshot changes.

#[test]
fn apply_file_watch_signal_changed_on_unchanged_disk_ignores_to_avoid_noise() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2aa_sig_unch_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap(); // ensure clean snapshot + possible "Saved." (but we set sentinel)
    assert!(!app.file.dirty);

    // Set a sentinel message that must be preserved (simulates "Saved." after real save)
    app.message = Some("Saved.".to_string());

    // Simulate a Changed signal for our own write (unchanged vs snapshot)
    let sig = crate::file::watcher::FileWatchSignal::Changed;
    let visible = crate::app::watch::apply_file_watch_signal(&mut app, sig);

    // Must be ignored: no overwrite of message, no arm, no mutation
    assert!(!visible, "watcher unchanged must report not visible");
    assert_eq!(
        app.message.as_deref(),
        Some("Saved."),
        "must not overwrite prior message with unchanged"
    );
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

// check_file_watcher_once (no-render seam) tests. Stable no-OS cases.
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
    let had = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap();
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
    let had = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap();
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

// Queued + render cases live in watcher_runtime.rs (uses the cfg(test) seam).

// Phase 2-ad: stale pending_reload cleared by watcher Unchanged/NoPath observations.
// When no pending: Unchanged/NoPath still fully ignored (no msg, no render).
// Manual Ctrl+R Unchanged/NoPath behavior is unchanged (always surfaces message).

#[test]
fn watcher_unchanged_clears_stale_pending_and_sets_message() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2ad_sig_unch_clr_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);

    // Simulate prior external mod arm (as if a watcher Changed had armed)
    std::fs::write(&p, "EXT").unwrap();
    let sig = crate::file::watcher::FileWatchSignal::Changed;
    let _ = crate::app::watch::apply_file_watch_signal(&mut app, sig);
    assert!(app.pending_reload.is_some(), "precondition: pending armed");

    // Revert disk content to match baseline snapshot's len; update snapshot mtime
    // so next observe sees Unchanged vs the *current known baseline state*.
    // (This exercises the watcher "resolution" branch without mtime syscalls.)
    std::fs::write(&p, "BASE").unwrap();
    // Refresh snapshot to the just-written state so observe classifies it Unchanged.
    if let Ok(s) = crate::file::io::capture_file_snapshot(std::path::Path::new(&p)) {
        app.file.disk_snapshot = Some(s);
    }

    app.message = Some("Saved.".to_string()); // sentinel that should be overwritten on resolution
    let before_dirty = app.file.dirty;

    let visible = crate::app::watch::apply_file_watch_signal(
        &mut app,
        crate::file::watcher::FileWatchSignal::Changed,
    );

    assert!(
        visible,
        "Unchanged with prior pending must be visible (clear + msg)"
    );
    assert!(
        app.pending_reload.is_none(),
        "stale pending must be cleared"
    );
    assert_eq!(
        app.message.as_deref(),
        Some("File unchanged on disk."),
        "must surface unchanged resolution message"
    );
    assert_eq!(app.file.dirty, before_dirty);
    assert_eq!(app.buffer.to_string(), "BASE"); // no reload of content

    let _ = std::fs::remove_file(&p);
}

#[test]
fn watcher_unchanged_with_no_pending_ignores_and_preserves_saved() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2ad_sig_unch_nopend_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);

    app.message = Some("Saved.".to_string());
    let before_pend = app.pending_reload.clone();

    // Disk is already at baseline; Changed -> observe Unchanged, no pending => ignore
    let visible = crate::app::watch::apply_file_watch_signal(
        &mut app,
        crate::file::watcher::FileWatchSignal::Changed,
    );

    assert!(!visible);
    assert_eq!(app.message.as_deref(), Some("Saved."));
    assert_eq!(app.pending_reload, before_pend);

    let _ = std::fs::remove_file(&p);
}

#[test]
fn watcher_nopath_with_pending_clears_it() {
    // NoPath observation while a watcher may be attached is possible if path is
    // cleared after watcher construction (or via direct helper test). We exercise
    // the apply seam directly: path=None + pending present => clear + msg + visible.
    let mut app = App::new(None).unwrap();
    // Force a path + watcher for realism of "had watcher", then drop the path.
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2ad_nopath_pend_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "X").unwrap();
    app.file.path = Some(std::path::PathBuf::from(&p));
    // Attach a test watcher for the path (lifecycle not under test here).
    let (tw, _tx) = crate::file::watcher::FileWatcher::new_for_test(app.file.path.clone().unwrap());
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw);

    // Arm a pending as if prior Modified had happened.
    app.pending_reload = Some(crate::app::reload::PendingReload {
        path: app.file.path.clone().unwrap(),
        status: crate::file::io::ExternalFileStatus::Modified,
        snapshot: app.file.disk_snapshot.clone(),
    });
    app.message = Some("prior".to_string());

    // Now remove path (simulates transition); apply a Changed signal.
    app.file.path = None;

    let visible = crate::app::watch::apply_file_watch_signal(
        &mut app,
        crate::file::watcher::FileWatchSignal::Changed,
    );

    assert!(
        visible,
        "NoPath with pending must report visible to render the resolution"
    );
    assert!(app.pending_reload.is_none());
    assert_eq!(app.message.as_deref(), Some("No file path."));

    let _ = std::fs::remove_file(&p);
}

#[test]
fn manual_ctrl_r_unchanged_shows_message_even_with_no_pending() {
    // Explicit coverage per 2-ad: manual path must always surface the message for
    // Unchanged, independent of pending state. (reload::apply_check_observation)
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2ad_man_unch_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "HELLO").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_reload.is_none());

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();

    assert_eq!(app.message.as_deref(), Some("File unchanged on disk."));
    assert!(app.pending_reload.is_none());

    let _ = std::fs::remove_file(&p);
}
