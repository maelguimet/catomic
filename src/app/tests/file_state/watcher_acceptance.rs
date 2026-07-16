//! Focused deterministic acceptance tests for watcher + manual Ctrl+R flows.
//!
//! Purpose: exercise automatic clean reload plus dirty/config-disabled Ctrl+R fallback.
//! Owns: deterministic watcher-to-reload acceptance cases.
//! Must not: change manual Ctrl+R or save conflict behavior; depend on live notify
//!   event timing; discard dirty content; introduce flakiness.
//! Invariants: clean default-on buffers reload immediately; dirty buffers never do;
//!   second Ctrl+R performs only on exact pending match; retarget coverage inspects
//!   watcher identities directly.
//! Phase: 2-af split through post-v0.1 watcher identity refresh.

use super::super::super::*;
use super::super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};

// --- Watcher + reload acceptance tests (deterministic seams) ---
// Prove clean automatic reload and dirty/config-disabled confirmation behavior.
// Uses TestStub + inject_signal only.

#[test]
fn watcher_changed_clean_buffer_auto_reloads() {
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

    // watcher sees Changed -> reloads the clean buffer immediately
    let path = app.file.path.clone().unwrap();
    let (tw, _tx) = crate::file::watcher::FileWatcher::new_for_test(path);
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw);
    app.file_watcher
        .as_ref()
        .expect("test watcher")
        .inject_signal(crate::file::watcher::FileWatchSignal::Changed);

    let mut out: Vec<u8> = Vec::new();
    let _ = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap();
    assert_eq!(
        app.buffer.to_string(),
        "EXTCONTENT",
        "buffer must be external content"
    );
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

#[cfg(unix)]
#[test]
fn symlink_retarget_reload_refreshes_watcher_to_new_referent() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!(
        "catomic_symlink_retarget_{}",
        std::process::id()
    ));
    let link_dir = root.join("links");
    let target_dir = root.join("targets");
    let link = link_dir.join("notes.txt");
    let first = target_dir.join("first.txt");
    let second = target_dir.join("second.txt");

    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&link_dir).unwrap();
    std::fs::create_dir_all(&target_dir).unwrap();
    std::fs::write(&first, "FIRST").unwrap();
    std::fs::write(&second, "SECOND-REFERENT").unwrap();
    symlink("../targets/first.txt", &link).unwrap();

    let mut app = App::new(Some(link.to_str().unwrap())).unwrap();
    let first = std::fs::canonicalize(&first).unwrap();
    assert!(
        app.file_watcher
            .as_ref()
            .expect("initial watcher")
            .watched_targets_for_test()
            .contains(&first)
    );

    std::fs::remove_file(&link).unwrap();
    symlink("../targets/second.txt", &link).unwrap();
    let observation = crate::file::io::observe_external_file(
        app.file.path.as_deref(),
        app.file.disk_snapshot.as_ref(),
    );
    assert_eq!(
        observation.status,
        crate::file::io::ExternalFileStatus::Modified
    );

    crate::app::reload::perform_observed_reload(&mut app, &observation);

    assert_eq!(app.buffer.to_string(), "SECOND-REFERENT");
    let second = std::fs::canonicalize(&second).unwrap();
    let targets = app
        .file_watcher
        .as_ref()
        .expect("refreshed watcher")
        .watched_targets_for_test();
    assert!(targets.contains(&second));
    assert!(!targets.contains(&first));

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn watcher_deleted_clean_buffer_auto_clears() {
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
    assert!(app.pending_reload.is_none());

    assert_eq!(
        app.buffer.to_string(),
        "",
        "buffer must be empty after deleted reload"
    );
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
    app.auto_reload = false;
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
