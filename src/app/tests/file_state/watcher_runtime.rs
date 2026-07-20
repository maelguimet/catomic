//! Deterministic queued-signal + render seam tests (Phase 2-ac).
//!
//! Purpose: exercise check_file_watcher_once_and_render (and the check/apply
//! chain) using the #[cfg(test)] injection seam. All tests are fully
//! deterministic; no live OS notify, no timing waits, no default-run flakiness.
//! Owns: the required queued Changed (modified), Deleted, unchanged-ignore,
//!   Error, and one-call-one-signal cases.
//! Must not: rely on real FS events; exercise or alter manual Ctrl+R semantics
//!   or save-conflict; read content except through existing reload path (not here).
//! Invariants: watcher signals are hints; unchanged/no-path are suppressed;
//!   clean Modified/Deleted auto-reload by default; at most one signal is consumed.
//! Phase: 2-ac through 2-bx automatic clean reload.

use super::super::super::*;
use crossterm::event::{KeyCode, KeyModifiers};

use super::super::make_key;

// Small helper to build a notify Event carrying a path (used only for tx
// injection in the one-at-a-time test to exercise map path + two queued).
fn make_modify_event(p: &std::path::Path) -> notify::Event {
    notify::Event {
        kind: notify::EventKind::Modify(notify::event::ModifyKind::Any),
        paths: vec![p.to_path_buf()],
        attrs: Default::default(),
    }
}

// queued Changed + externally modified => visible arm + render
#[test]
fn queued_changed_external_modified_auto_reloads_and_renders() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2ac_q_mod_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "ORIG").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    // clean snapshot
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);

    // external change (so observe will report Modified)
    std::fs::write(&p, "ORIGEXT").unwrap();

    // replace with injectable test watcher (no live notify)
    let path = app.file.path.clone().unwrap();
    let (tw, _tx) = crate::file::watcher::FileWatcher::new_for_test(path);
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw);
    app.file_watcher
        .as_ref()
        .expect("test watcher installed")
        .inject_signal(crate::file::watcher::FileWatchSignal::Changed);

    // sentinel to prove we don't clobber random prior msg
    app.message = Some("Saved.".to_string());

    let mut out: Vec<u8> = Vec::new();
    let had = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap();

    assert!(had, "should report handled for visible Modified");
    assert!(!out.is_empty(), "must have rendered");
    assert!(app.pending_reload.is_none());
    assert_eq!(app.message.as_deref(), Some("Reloaded from disk."));
    assert_eq!(app.buffer.to_string(), "ORIGEXT");
    assert!(!app.file.dirty, "dirty must be unchanged");

    let _ = std::fs::remove_file(&p);
}

// queued Deleted + file deleted => visible + render
#[test]
fn queued_deleted_external_delete_auto_clears_and_renders() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2ac_q_del_{}.txt", std::process::id()));
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
    let had = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap();

    assert!(had);
    assert!(!out.is_empty());
    assert!(app.pending_reload.is_none());
    assert_eq!(
        app.message.as_deref(),
        Some("Buffer cleared (file deleted on disk).")
    );
    assert_eq!(app.buffer.to_string(), "");
    assert!(!app.file.dirty);

    // recreate for cleanup
    std::fs::write(&p, "TODEL").unwrap();
    let _ = std::fs::remove_file(&p);
}

// queued Changed + disk unchanged => ignored, no render, prior message preserved
#[test]
fn queued_changed_on_unchanged_ignored_no_render() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2ac_q_unch_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);

    // sentinel that must survive
    app.message = Some("Saved.".to_string());
    let before_pend = app.pending_reload.clone();

    let path = app.file.path.clone().unwrap();
    let (tw, _tx) = crate::file::watcher::FileWatcher::new_for_test(path);
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw);
    app.file_watcher
        .as_ref()
        .expect("test watcher")
        .inject_signal(crate::file::watcher::FileWatchSignal::Changed);

    let mut out: Vec<u8> = Vec::new();
    let had = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap();

    assert!(!had, "unchanged watcher signal must not report handled");
    assert!(out.is_empty(), "must not render on ignored");
    assert_eq!(
        app.message.as_deref(),
        Some("Saved."),
        "must preserve prior message"
    );
    assert_eq!(app.pending_reload, before_pend);
    assert_eq!(app.buffer.to_string(), "BASE\n");
    assert!(!app.file.dirty);

    let _ = std::fs::remove_file(&p);
}

// queued Error => visible error message + render, no state mutation
#[test]
fn queued_error_is_visible_and_does_not_mutate() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2ac_q_err_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "EBASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    let before_buf = app.buffer.to_string();
    let before_dirty = app.file.dirty;
    let before_snap = app.file.disk_snapshot.clone();

    let path = app.file.path.clone().unwrap();
    let (tw, _tx) = crate::file::watcher::FileWatcher::new_for_test(path);
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw);
    app.file_watcher
        .as_ref()
        .expect("test watcher")
        .inject_signal(crate::file::watcher::FileWatchSignal::Error(
            "boom-deterministic".to_string(),
        ));

    let mut out: Vec<u8> = Vec::new();
    let had = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap();

    assert!(had);
    assert!(!out.is_empty());
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.starts_with("File watcher error:"),
        "error msg prefix required: got {:?}",
        app.message
    );
    assert_eq!(app.buffer.to_string(), before_buf);
    assert_eq!(app.file.dirty, before_dirty);
    assert_eq!(app.file.disk_snapshot, before_snap);

    let _ = std::fs::remove_file(&p);
}

// one call processes at most one queued signal (even if two are present).
// Deterministic mpsc seam: two visible Modified signals must each be
// consumed by a separate call (at most one per call). Both produce visible
// outcome (arm/refresh message + render) because watcher path does not
// update the disk_snapshot used by observe.
#[test]
fn one_call_processes_at_most_one_signal() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2ac_q_one_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "ONE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.auto_reload = false;
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    // external mod makes observe report Modified for both signals
    // (disk_snapshot is not mutated by the watcher arm path)
    std::fs::write(&p, "ONEEXT").unwrap();

    let path = app.file.path.clone().unwrap();
    let (tw, tx) = crate::file::watcher::FileWatcher::new_for_test(path.clone());
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw);

    // Queue two raw relevant events through the test channel (not live notify).
    let ev = make_modify_event(&path);
    let _ = tx.send(Ok(ev.clone()));
    let _ = tx.send(Ok(ev));

    // First call consumes at most one.
    let mut out1: Vec<u8> = Vec::new();
    let r1 = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out1).unwrap();
    assert!(r1, "first signal must return true (visible Modified)");
    assert!(!out1.is_empty(), "first must have rendered");

    // Second call must still be able to consume the second queued signal.
    let mut out2: Vec<u8> = Vec::new();
    let r2 = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out2).unwrap();
    assert!(
        r2,
        "second signal must return true (second visible Modified)"
    );
    assert!(!out2.is_empty(), "second must have rendered");

    // State remains sane; no content mutation; both signals were observable.
    assert_eq!(app.buffer.to_string(), "ONE\n", "buffer content unchanged");
    assert!(!app.file.dirty, "dirty state sane");
    // pending may be set (or re-set); message from arm path is present.
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("changed on disk") || msg.contains("Ctrl+R"),
        "message should reflect arm from one of the visible signals: got {:?}",
        app.message
    );

    let _ = std::fs::remove_file(&p);
}
