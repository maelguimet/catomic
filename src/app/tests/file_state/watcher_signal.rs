//! Focused deterministic tests for watcher signal application and check seam.
//!
//! Purpose: exercise automatic clean reload, dirty fallback, and watcher suppression.
//! Owns: direct apply + simple check seam tests.
//! Must not: rely on live OS notify delivery or change manual Ctrl+R semantics.
//! Invariants: clean Modified/Deleted reload immediately when enabled; dirty buffers
//!   retain a passive observation; Unchanged/NoPath observations are ignored when no
//!   pending exists.

use super::super::super::*;
use super::super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};

#[test]
fn apply_file_watch_signal_changed_on_unchanged_disk_ignores_to_avoid_noise() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2aa_sig_unch_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);

    app.message_warning("Prior warning.");

    let sig = crate::file::watcher::FileWatchSignal::Changed;
    let visible = crate::app::watch::apply_file_watch_signal(&mut app, sig);

    assert!(!visible, "watcher unchanged must report not visible");
    assert_eq!(
        app.message.as_deref(),
        Some("Prior warning."),
        "must not overwrite prior message with unchanged"
    );
    assert!(app.observed_change.is_none());
    assert_eq!(app.buffer.to_string(), "BASE");
    assert!(!app.file.dirty);

    let _ = std::fs::remove_file(&p);
}

#[test]
fn apply_file_watch_signal_changed_clean_buffer_auto_reloads() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2aa_sig_mod_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "ORIG").unwrap();

    let mut app = App::new(Some(&p)).unwrap();

    std::fs::write(&p, "ORIGEXT").unwrap();

    let sig = crate::file::watcher::FileWatchSignal::Changed;
    crate::app::watch::apply_file_watch_signal(&mut app, sig);

    assert!(app.message.is_none());
    assert!(app.observed_change.is_none());
    assert_eq!(app.buffer.to_string(), "ORIGEXT");
    assert!(!app.file.dirty);

    let _ = std::fs::remove_file(&p);
}

#[test]
fn apply_file_watch_signal_changed_dirty_external_warns_without_explicit_arm() {
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
        .unwrap();
    assert!(app.file.dirty);

    std::fs::write(&p, "BASEEXT").unwrap();

    let sig = crate::file::watcher::FileWatchSignal::Changed;
    crate::app::watch::apply_file_watch_signal(&mut app, sig);

    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("changed on disk") && msg.contains("discard"),
        "dirty external Modified must warn about discard: got {:?}",
        app.message
    );
    assert!(
        app.observed_change.is_some() && app.confirmation.armed_reload().is_none(),
        "passive watcher warning must not arm destructive reload"
    );
    assert!(app.file.dirty, "must not clear dirty");
    assert_eq!(app.buffer.to_string(), "xBASE");

    let _ = std::fs::remove_file(&p);
}

#[test]
fn watcher_does_not_hide_matching_save_overwrite_confirmation() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_watcher_save_confirmation_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    std::fs::write(&p, "EXTERNAL").unwrap();

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.confirmation.save_conflict().is_some());
    assert_eq!(
        app.message.as_deref(),
        Some("File changed on disk. Press Ctrl+S again to overwrite.")
    );

    let visible = crate::app::watch::apply_file_watch_signal(
        &mut app,
        crate::file::watcher::FileWatchSignal::Changed,
    );

    assert!(visible);
    assert!(app.confirmation.save_conflict().is_some());
    assert!(app.observed_change.is_some() && app.confirmation.armed_reload().is_none());
    assert_eq!(
        app.message.as_deref(),
        Some("File changed on disk. Press Ctrl+S again to overwrite."),
        "the visible warning must describe the still-armed destructive save"
    );
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "EXTERNAL");

    let _ = std::fs::remove_file(&p);
}

#[test]
fn apply_file_watch_signal_deleted_clean_buffer_auto_clears() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2aa_sig_del_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "TODEL").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);

    let _ = std::fs::remove_file(&p);

    let sig = crate::file::watcher::FileWatchSignal::Deleted;
    crate::app::watch::apply_file_watch_signal(&mut app, sig);

    assert_eq!(app.message.as_deref(), None);
    assert!(app.observed_change.is_none());
    assert_eq!(app.buffer.to_string(), "");
    assert!(!app.file.dirty);

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

    let _ = std::fs::remove_file(&p);
}

#[test]
fn check_file_watcher_once_no_watcher_returns_false_no_mutation() {
    let mut app = App::new(None).unwrap();
    assert!(app.file_watcher.is_none());

    let before_msg = app.message.clone();
    let before_pend = (app.observed_change.clone(), app.confirmation.clone());
    let before_dirty = app.file.dirty;

    let had = crate::app::watch::check_file_watcher_once(&mut app);
    assert!(!had);
    assert_eq!(app.message, before_msg);
    assert_eq!(
        (app.observed_change.clone(), app.confirmation.clone()),
        before_pend
    );
    assert_eq!(app.file.dirty, before_dirty);
}

#[test]
fn check_file_watcher_once_with_watcher_no_signal_returns_false_no_mutation() {
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
    let before_pend = (app.observed_change.clone(), app.confirmation.clone());

    let had = crate::app::watch::check_file_watcher_once(&mut app);
    assert!(
        !had,
        "no queued signal expected immediately after construct"
    );
    assert_eq!(app.message, before_msg);
    assert_eq!(
        (app.observed_change.clone(), app.confirmation.clone()),
        before_pend
    );

    let _ = std::fs::remove_file(&p);
}

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
