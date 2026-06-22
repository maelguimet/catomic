//! Focused tests for watcher-observed stale pending cleanup and watcher-armed + manual follow-up.
//!
//! Purpose: exercise Unchanged/NoPath watcher signals clearing armed pending_reload,
//!   and (later) watcher-armed pending followed by manual Ctrl+R confirmation.
//! Owns: stale-pending resolution via watcher path; watcher + manual reload integration tests
//!   (using deterministic queued seams).
//! Must not: rely on live OS notify; change manual Ctrl+R semantics or save conflict;
//!   read content except through the confirmed reload path; add auto-reload.
//! Invariants: watcher signals are hints only; Unchanged/NoPath clear pending only when armed
//!   (otherwise ignored to suppress self-save noise); manual second Ctrl+R performs the reload
//!   using the same observe + pending match logic; no behavior change on split.
//! Phase: 2-ae split + acceptance (deterministic seams).

use super::super::super::*;
use super::super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};

// Stale pending cleanup via watcher observations (Unchanged/NoPath).
// These were originally in watcher_signal.rs under 2-ad.

// When a prior watcher Changed armed pending, and disk reverts to match baseline,
// a subsequent watcher Changed observes Unchanged and clears the stale pending.
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

    app.message = Some("Saved.".to_string());

    let mut out1: Vec<u8> = Vec::new();
    let r1 = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out1).unwrap();
    assert!(r1, "first Changed+Modified must be visible and arm");
    assert!(app.pending_reload.is_some());
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
    assert_eq!(app.message.as_deref(), Some("File unchanged on disk."));
    assert!(!out2.is_empty(), "must render on resolution");
    assert_eq!(
        app.buffer.to_string(),
        "ORIG",
        "content must not have reloaded"
    );

    let _ = std::fs::remove_file(&p);
}

// Manual Ctrl+R Unchanged behavior is independent of watcher path.
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

// --- Phase 2-ae: watcher + manual Ctrl+R acceptance tests (deterministic seams) ---
// Prove watcher arm integrates with existing manual confirmation path.
// No auto-reload; second Ctrl+R performs via the confirmed reload path.
// Uses TestStub + inject_signal only.

#[test]
fn watcher_changed_external_modified_then_manual_ctrl_r_second_reloads() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2ae_w_mod_reload_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "ORIG").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);

    // external modification
    std::fs::write(&p, "EXTCONTENT").unwrap();

    // watcher sees Changed -> arms (via seam)
    let path = app.file.path.clone().unwrap();
    let (tw, _tx) = crate::file::watcher::FileWatcher::new_for_test(path);
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw);
    app.file_watcher
        .as_ref()
        .expect("test watcher")
        .inject_signal(crate::file::watcher::FileWatchSignal::Changed);

    let mut out: Vec<u8> = Vec::new();
    let _ = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap();
    assert!(app.pending_reload.is_some(), "watcher must arm pending");
    let msg = app.message.as_deref().unwrap_or("");
    assert!(msg.contains("changed on disk") && msg.contains("Ctrl+R again"));

    // second Ctrl+R: performs reload
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "EXTCONTENT", "buffer must be external content");
    assert!(!app.file.dirty, "dirty must be false after reload");
    match &app.file.disk_snapshot {
        Some(crate::file::io::FileSnapshot::Present { .. }) => {}
        _ => panic!("disk_snapshot must be Present after reload"),
    }
    assert_eq!(
        app.message.as_deref(),
        Some("Reloaded from disk."),
        "must show reload success"
    );
    assert!(app.pending_reload.is_none());

    let _ = std::fs::remove_file(&p);
}

#[test]
fn watcher_deleted_then_manual_ctrl_r_second_clears_buffer() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2ae_w_del_clear_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "TODEL").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);

    let _ = std::fs::remove_file(&p); // external delete

    let path = app.file.path.clone().unwrap();
    let (tw, _tx) = crate::file::watcher::FileWatcher::new_for_test(path);
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw);
    app.file_watcher
        .as_ref()
        .expect("test watcher")
        .inject_signal(crate::file::watcher::FileWatchSignal::Deleted);

    let mut out: Vec<u8> = Vec::new();
    let _ = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap();
    assert!(app.pending_reload.is_some());

    // second Ctrl+R: clears
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "", "buffer must be empty after deleted reload");
    assert!(!app.file.dirty);
    assert_eq!(
        app.file.disk_snapshot,
        Some(crate::file::io::FileSnapshot::Absent),
        "snapshot must be Absent after clear"
    );
    assert_eq!(
        app.message.as_deref(),
        Some("Buffer cleared (file deleted on disk).")
    );
    assert!(app.pending_reload.is_none());

    // recreate for cleanup hygiene
    std::fs::write(&p, "TODEL").unwrap();
    let _ = std::fs::remove_file(&p);
}

#[test]
fn watcher_changed_dirty_external_arms_discard_warning_then_ctrl_r_reloads() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2ae_w_dirty_discard_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap(); // local dirty
    assert!(app.file.dirty);

    std::fs::write(&p, "BASEEXT").unwrap(); // external

    let path = app.file.path.clone().unwrap();
    let (tw, _tx) = crate::file::watcher::FileWatcher::new_for_test(path);
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw);
    app.file_watcher
        .as_ref()
        .expect("test watcher")
        .inject_signal(crate::file::watcher::FileWatchSignal::Changed);

    let mut out: Vec<u8> = Vec::new();
    let _ = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap();

    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("changed on disk") && msg.contains("discard"),
        "watcher arm on dirty must include discard warning: got {:?}",
        app.message
    );
    assert!(app.pending_reload.is_some());

    // second R reloads, discards local
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "BASEEXT");
    assert!(!app.file.dirty);
    assert!(app.pending_reload.is_none());

    let _ = std::fs::remove_file(&p);
}

#[test]
fn watcher_armed_pending_local_edit_clears_then_next_ctrl_r_rearms() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2ae_w_edit_clears_pend_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "EBASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    std::fs::write(&p, "EMOD").unwrap();

    // watcher arms
    let path = app.file.path.clone().unwrap();
    let (tw, _tx) = crate::file::watcher::FileWatcher::new_for_test(path);
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw);
    app.file_watcher
        .as_ref()
        .expect("test watcher")
        .inject_signal(crate::file::watcher::FileWatchSignal::Changed);

    let mut out: Vec<u8> = Vec::new();
    let _ = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap();
    assert!(app.pending_reload.is_some());

    // local edit clears pending (no reload)
    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.pending_reload.is_none());

    // next Ctrl+R must re-arm (first press behavior), not auto-reload
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(
        app.pending_reload.is_some(),
        "after local edit clears watcher-pending, next Ctrl+R must re-arm"
    );
    // buffer still has the 'z' edit, not external
    assert!(app.buffer.to_string().contains('z') || app.buffer.to_string().contains("EBASE"));

    let _ = std::fs::remove_file(&p);
}

// (watcher-armed + successful save clear of pending_reload is exercised at code level in
// save.rs:do_atomic_save and is not required as a new acceptance case here per "add only if
// not already covered"; save-conflict semantics are untouched.)
