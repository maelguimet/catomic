//! Focused tests for watcher-observed stale pending cleanup only.
//!
//! Purpose: exercise Unchanged/NoPath watcher signals clearing armed pending_reload.
//! Owns: stale-pending resolution via watcher path (deterministic seams only).
//! Must not: contain acceptance/manual-ctrl-r follow-up tests (see watcher_acceptance);
//!   rely on live OS notify; change any reload/save semantics; read content.
//! Invariants: watcher signals are hints only; Unchanged/NoPath clear only when armed
//!   (else ignored to suppress noise); no behavior change.
//! Phase: 2-af (split hygiene; no behavior change).

use super::super::super::*;
use super::super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};

// Stale pending cleanup via watcher observations (Unchanged/NoPath).
// These were originally in watcher_signal.rs under 2-ad.

// When a prior watcher Changed armed pending, and disk reverts to match baseline,
// a subsequent watcher Changed observes Unchanged and clears the stale pending.
#[test]
fn watcher_unchanged_clears_stale_pending_and_restores_status() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2ad_sig_unch_clr_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.auto_reload = false;
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

    app.message_warning("Prior warning."); // sentinel cleared on resolution
    let before_dirty = app.file.dirty;

    let visible = crate::app::watch::apply_file_watch_signal(
        &mut app,
        crate::file::watcher::FileWatchSignal::Changed,
    );

    assert!(
        visible,
        "Unchanged with prior pending must visibly restore normal status"
    );
    assert!(
        app.pending_reload.is_none(),
        "stale pending must be cleared"
    );
    assert_eq!(app.message.as_deref(), None, "must restore normal status");
    assert_eq!(app.file.dirty, before_dirty);
    assert_eq!(app.buffer.to_string(), "BASE\n"); // no reload of content

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

    app.message_warning("Prior warning.");
    let before_pend = app.pending_reload.clone();

    // Disk is already at baseline; Changed -> observe Unchanged, no pending => ignore
    let visible = crate::app::watch::apply_file_watch_signal(
        &mut app,
        crate::file::watcher::FileWatchSignal::Changed,
    );

    assert!(!visible);
    assert_eq!(app.message.as_deref(), Some("Prior warning."));
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
    app.message_warning("prior");

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

// Queued (deterministic seam) variant of watcher-observed Unchanged clearing stale pending.
// Moved here for "watcher pending" focus; exercises the render helper seam.
#[test]
fn queued_changed_then_unchanged_clears_stale_pending_and_renders() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2ad_q_unch_clr_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "ORIG").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.auto_reload = false;
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);

    // external change -> will be Modified on observe
    std::fs::write(&p, "ORIGEXT").unwrap();

    let path = app.file.path.clone().unwrap();
    let (tw, _tx) = crate::file::watcher::FileWatcher::new_for_test(path.clone());
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw);
    app.file_watcher
        .as_ref()
        .expect("test watcher")
        .inject_signal(crate::file::watcher::FileWatchSignal::Changed);

    app.message_warning("Prior warning.");

    let mut out1: Vec<u8> = Vec::new();
    let r1 = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out1).unwrap();
    assert!(r1, "first Changed+Modified must be visible and arm");
    assert!(app.pending_reload.is_some());
    assert_eq!(
        app.message_role,
        crate::terminal::render::StatusRole::Warning
    );
    assert!(!out1.is_empty());

    // Revert on disk to original content; refresh snapshot so next observe is Unchanged.
    std::fs::write(&p, "ORIG").unwrap();
    if let Ok(s) = crate::file::io::capture_file_snapshot(std::path::Path::new(&p)) {
        app.file.disk_snapshot = Some(s);
    }

    // Second watcher Changed now observes Unchanged vs (updated) baseline.
    app.file_watcher
        .as_ref()
        .expect("test watcher")
        .inject_signal(crate::file::watcher::FileWatchSignal::Changed);

    let mut out2: Vec<u8> = Vec::new();
    let r2 = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out2).unwrap();

    assert!(
        r2,
        "Unchanged observation with stale pending must be visible and render"
    );
    assert!(app.pending_reload.is_none(), "pending must be cleared");
    assert!(app.message.is_none());
    assert!(!out2.is_empty(), "must render on resolution");
    assert_eq!(
        app.buffer.to_string(),
        "ORIG\n",
        "content must not have reloaded"
    );

    let _ = std::fs::remove_file(&p);
}

// Manual Ctrl+R Unchanged behavior is independent of watcher path.
#[test]
fn manual_ctrl_r_unchanged_restores_normal_status() {
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

    assert!(app.message.is_none());
    assert!(app.pending_reload.is_none());

    let _ = std::fs::remove_file(&p);
}
