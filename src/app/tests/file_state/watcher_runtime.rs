//! Deterministic queued-signal and render-seam tests.
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

use super::super::super::*;
use crossterm::event::{KeyCode, KeyModifiers};

use super::super::make_key;

#[test]
fn queued_changed_external_modified_auto_reloads_and_renders() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2ac_q_mod_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "ORIG").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);

    std::fs::write(&p, "ORIGEXT").unwrap();

    let path = app.file.path.clone().unwrap();
    let (tw, _tx) = crate::file::watcher::FileWatcher::new_for_test(path);
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw);
    app.file_watcher
        .as_ref()
        .expect("test watcher installed")
        .inject_signal(crate::file::watcher::FileWatchSignal::Changed);

    app.message_warning("Prior warning.");

    let mut out: Vec<u8> = Vec::new();
    let had = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap();

    assert!(had, "should report handled for visible Modified");
    assert!(!out.is_empty(), "must have rendered");
    assert!(app.pending_reload.is_none());
    assert!(app.message.is_none());
    assert_eq!(app.buffer.to_string(), "ORIGEXT");
    assert!(!app.file.dirty, "dirty must be unchanged");

    let _ = std::fs::remove_file(&p);
}

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

    let _ = std::fs::remove_file(&p);

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
    assert_eq!(app.message.as_deref(), None);
    assert_eq!(app.buffer.to_string(), "");
    assert!(!app.file.dirty);

    std::fs::write(&p, "TODEL").unwrap();
    let _ = std::fs::remove_file(&p);
}

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

    app.message_warning("Prior warning.");
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
        Some("Prior warning."),
        "must preserve prior message"
    );
    assert_eq!(app.pending_reload, before_pend);
    assert_eq!(app.buffer.to_string(), "BASE");
    assert!(!app.file.dirty);

    let _ = std::fs::remove_file(&p);
}

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
    std::fs::write(&p, "ONEEXT").unwrap();

    let path = app.file.path.clone().unwrap();
    let (tw, tx) = crate::file::watcher::FileWatcher::new_for_test(path.clone());
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw);

    let signal = crate::file::watcher::FileWatchSignal::Changed;
    let _ = tx.send(signal.clone());
    let _ = tx.send(signal);

    let mut out1: Vec<u8> = Vec::new();
    let r1 = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out1).unwrap();
    assert!(r1, "first signal must return true (visible Modified)");
    assert!(!out1.is_empty(), "first must have rendered");

    let mut out2: Vec<u8> = Vec::new();
    let r2 = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out2).unwrap();
    assert!(
        r2,
        "second signal must return true (second visible Modified)"
    );
    assert!(!out2.is_empty(), "second must have rendered");

    assert_eq!(app.buffer.to_string(), "ONE", "buffer content unchanged");
    assert!(!app.file.dirty, "dirty state sane");
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("changed on disk") || msg.contains("Ctrl+R"),
        "message should reflect arm from one of the visible signals: got {:?}",
        app.message
    );

    let _ = std::fs::remove_file(&p);
}

fn app_with_clean_file(label: &str, text: &str) -> (App, String) {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_watch_stop_{label}_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, text).unwrap();
    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);
    (app, p)
}

#[test]
fn stopped_watcher_is_restarted_and_reported() {
    let (mut app, p) = app_with_clean_file("restart", "ORIG");
    let (tw, tx) = crate::file::watcher::FileWatcher::new_for_test(app.file.path.clone().unwrap());
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw);
    drop(tx);

    let mut out: Vec<u8> = Vec::new();
    let had = crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap();

    assert!(had, "a stopped watcher is a visible event");
    assert_eq!(
        app.message.as_deref(),
        Some(crate::app::watch::WATCHER_RESTARTED)
    );
    let watcher = app.file_watcher.as_ref().expect("watcher restarted");
    assert_eq!(watcher.restarts(), 1);
    assert!(
        !crate::app::watch::check_file_watcher_once(&mut app),
        "the replacement watcher is live and idle"
    );
    let _ = std::fs::remove_file(&p);
}

#[test]
fn stopped_watcher_catches_up_on_a_change_missed_while_it_was_dead() {
    let (mut app, p) = app_with_clean_file("catch_up", "ORIG");
    let (tw, tx) = crate::file::watcher::FileWatcher::new_for_test(app.file.path.clone().unwrap());
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw);
    drop(tx);
    std::fs::write(&p, "CHANGED WHILE DEAD").unwrap();

    assert!(crate::app::watch::check_file_watcher_once(&mut app));

    assert_eq!(app.buffer.to_string(), "CHANGED WHILE DEAD");
    assert!(app.file_watcher.is_some());
    let _ = std::fs::remove_file(&p);
}

#[test]
fn repeatedly_stopping_watcher_gives_up_with_a_persistent_error() {
    let (mut app, p) = app_with_clean_file("give_up", "ORIG");
    let (tw, tx) = crate::file::watcher::FileWatcher::new_for_test(app.file.path.clone().unwrap());
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw.with_restarts(3));
    drop(tx);

    assert!(crate::app::watch::check_file_watcher_once(&mut app));

    assert!(app.file_watcher.is_none());
    assert_eq!(
        app.message.as_deref(),
        Some(crate::app::watch::WATCHER_STOPPED)
    );
    let _ = std::fs::remove_file(&p);
}

#[test]
fn catch_up_reload_keeps_the_restart_count() {
    let (mut app, p) = app_with_clean_file("keep_count", "ORIG");
    let (tw, tx) = crate::file::watcher::FileWatcher::new_for_test(app.file.path.clone().unwrap());
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw.with_restarts(2));
    drop(tx);
    std::fs::write(&p, "CHANGED WHILE DEAD").unwrap();

    assert!(crate::app::watch::check_file_watcher_once(&mut app));

    assert_eq!(app.buffer.to_string(), "CHANGED WHILE DEAD");
    let watcher = app.file_watcher.as_ref().expect("watcher restarted");
    assert_eq!(watcher.restarts(), 3);
    let _ = std::fs::remove_file(&p);
}

#[test]
fn giving_up_sticks_even_when_the_catch_up_observation_reloads() {
    let (mut app, p) = app_with_clean_file("give_up_reload", "ORIG");
    let (tw, tx) = crate::file::watcher::FileWatcher::new_for_test(app.file.path.clone().unwrap());
    crate::app::watch::replace_file_watcher_for_test(&mut app, tw.with_restarts(3));
    drop(tx);
    std::fs::write(&p, "CHANGED WHILE DEAD").unwrap();

    assert!(crate::app::watch::check_file_watcher_once(&mut app));

    assert_eq!(app.buffer.to_string(), "CHANGED WHILE DEAD");
    assert!(app.file_watcher.is_none());
    assert_eq!(
        app.message.as_deref(),
        Some(crate::app::watch::WATCHER_STOPPED)
    );
    let _ = std::fs::remove_file(&p);
}
