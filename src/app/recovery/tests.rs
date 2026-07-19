//! Purpose: prove opt-in catnap timing, preview, drift refusal, undo, and save cleanup.
//! Owns: deterministic App-level recovery tests using temporary sibling sidecars.
//! Must not: sleep for configured intervals, use network, panic the process, or touch user files.
//! Invariants: source files stay unchanged until ordinary save; recovery applies only on Enter.
//! Phase: 8 recovery acceptance.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::config::cat::RecoveryConfig;

fn path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "catomic_app_recovery_{}_{}",
        std::process::id(),
        name
    ))
}

fn cleanup(original: &PathBuf) {
    let _ = std::fs::remove_file(original);
    let _ = std::fs::remove_file(crate::file::recovery::catnap_path(original));
}

fn enabled(app: &mut super::super::App, max_bytes: usize) {
    app.cat_config.recovery = RecoveryConfig {
        enabled: true,
        interval_secs: 5,
        max_bytes,
    };
}

fn dirty_insert(app: &mut super::super::App, ch: char) {
    app.buffer.insert_char(ch);
    super::super::file_state::refresh_dirty(&mut app.file, &*app.buffer);
}

fn force_due(app: &mut super::super::App) {
    app.recovery.last_attempt = Instant::now() - Duration::from_secs(6);
}

fn wait_for_catnap(app: &mut super::super::App, out: &mut Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.recovery.last_written_history.is_none() {
        poll(app, out).unwrap();
        assert!(Instant::now() < deadline, "catnap worker timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn default_disabled_recovery_never_writes() {
    let original = path("disabled.txt");
    cleanup(&original);
    std::fs::write(&original, "base").unwrap();
    let mut app = super::super::App::new(original.to_str()).unwrap();
    dirty_insert(&mut app, 'x');
    force_due(&mut app);

    poll(&mut app, &mut Vec::new()).unwrap();

    assert!(app.recovery.task.is_none());
    assert!(!crate::file::recovery::catnap_path(&original).exists());
    cleanup(&original);
}

#[test]
fn due_autosave_writes_only_the_private_sidecar() {
    let original = path("autosave.txt");
    cleanup(&original);
    std::fs::write(&original, "base").unwrap();
    let mut app = super::super::App::new(original.to_str()).unwrap();
    enabled(&mut app, 1024);
    dirty_insert(&mut app, 'x');
    force_due(&mut app);
    let mut out = Vec::new();

    poll(&mut app, &mut out).unwrap();
    wait_for_catnap(&mut app, &mut out);

    assert_eq!(std::fs::read_to_string(&original).unwrap(), "base");
    assert_eq!(
        std::fs::read_to_string(crate::file::recovery::catnap_path(&original)).unwrap(),
        "xbase"
    );
    cleanup(&original);
}

#[test]
fn oversized_buffer_is_skipped_without_starting_a_task() {
    let original = path("oversized.txt");
    cleanup(&original);
    std::fs::write(&original, "base").unwrap();
    let mut app = super::super::App::new(original.to_str()).unwrap();
    enabled(&mut app, 4);
    dirty_insert(&mut app, 'x');
    force_due(&mut app);

    poll(&mut app, &mut Vec::new()).unwrap();

    assert!(app.recovery.task.is_none());
    assert!(!crate::file::recovery::catnap_path(&original).exists());
    cleanup(&original);
}

#[test]
fn recovery_previews_then_applies_as_one_undoable_edit() {
    let original = path("preview.txt");
    cleanup(&original);
    std::fs::write(&original, "disk").unwrap();
    let mut app = super::super::App::new(original.to_str()).unwrap();
    enabled(&mut app, 1024);
    crate::file::io::atomic_write_private_string(
        crate::file::recovery::catnap_path(&original),
        "recovered",
    )
    .unwrap();
    let mut out = Vec::new();

    start_preview(&mut app, &mut out).unwrap();
    assert!(is_viewing(&app));
    assert_eq!(app.buffer.to_string(), "disk");
    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .unwrap();

    assert_eq!(app.buffer.to_string(), "recovered");
    assert_eq!(std::fs::read_to_string(&original).unwrap(), "disk");
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "disk");
    cleanup(&original);
}

#[test]
fn source_edit_during_preview_refuses_recovery() {
    let original = path("stale.txt");
    cleanup(&original);
    std::fs::write(&original, "disk").unwrap();
    let mut app = super::super::App::new(original.to_str()).unwrap();
    enabled(&mut app, 1024);
    crate::file::io::atomic_write_private_string(
        crate::file::recovery::catnap_path(&original),
        "recovered",
    )
    .unwrap();
    let mut out = Vec::new();

    start_preview(&mut app, &mut out).unwrap();
    dirty_insert(&mut app, 'x');
    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .unwrap();

    assert_eq!(app.buffer.to_string(), "xdisk");
    assert!(app.message.as_deref().unwrap().contains("Source changed"));
    cleanup(&original);
}

#[cfg(unix)]
#[test]
fn sidecar_replacement_after_startup_offer_refuses_preview() {
    let original = path("offered_sidecar_drift.txt");
    let replacement = path("offered_sidecar_drift_replacement.txt.catnap");
    cleanup(&original);
    let _ = std::fs::remove_file(&replacement);
    std::fs::write(&original, "disk").unwrap();
    let mut app = super::super::App::new(original.to_str()).unwrap();
    enabled(&mut app, 1024);
    let sidecar = crate::file::recovery::catnap_path(&original);
    crate::file::io::atomic_write_private_string(&sidecar, "recovered").unwrap();
    initialize(&mut app);
    assert!(app.message.as_deref().unwrap().contains("recovery found"));

    std::fs::write(&replacement, "recovered").unwrap();
    std::fs::rename(&replacement, &sidecar).unwrap();
    start_preview(&mut app, &mut Vec::new()).unwrap();

    assert!(!is_viewing(&app));
    assert_eq!(app.buffer.to_string(), "disk");
    assert!(app.message.as_deref().unwrap().contains("No newer catnap"));
    cleanup(&original);
    let _ = std::fs::remove_file(replacement);
}

#[cfg(unix)]
#[test]
fn sidecar_replacement_during_preview_refuses_recovery() {
    let original = path("sidecar_drift.txt");
    let replacement = path("sidecar_drift_replacement.txt.catnap");
    cleanup(&original);
    let _ = std::fs::remove_file(&replacement);
    std::fs::write(&original, "disk").unwrap();
    let mut app = super::super::App::new(original.to_str()).unwrap();
    enabled(&mut app, 1024);
    let sidecar = crate::file::recovery::catnap_path(&original);
    crate::file::io::atomic_write_private_string(&sidecar, "recovered").unwrap();
    let mut out = Vec::new();

    start_preview(&mut app, &mut out).unwrap();
    std::fs::write(&replacement, "recovered").unwrap();
    std::fs::rename(&replacement, &sidecar).unwrap();
    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .unwrap();

    assert_eq!(app.buffer.to_string(), "disk");
    assert!(app.message.as_deref().unwrap().contains("Catnap changed"));
    cleanup(&original);
    let _ = std::fs::remove_file(replacement);
}

#[test]
fn malformed_sidecar_reports_an_error_without_ending_the_editor() {
    let original = path("invalid_utf8.txt");
    cleanup(&original);
    std::fs::write(&original, "disk").unwrap();
    let mut app = super::super::App::new(original.to_str()).unwrap();
    enabled(&mut app, 1024);
    std::fs::write(crate::file::recovery::catnap_path(&original), [0xff]).unwrap();

    start_preview(&mut app, &mut Vec::new()).unwrap();

    assert!(!is_viewing(&app));
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("Cannot open catnap"));
    cleanup(&original);
}

#[test]
fn successful_save_waits_for_and_removes_catnap() {
    let original = path("save_cleanup.txt");
    cleanup(&original);
    std::fs::write(&original, "base").unwrap();
    let mut app = super::super::App::new(original.to_str()).unwrap();
    enabled(&mut app, 1024);
    dirty_insert(&mut app, 'x');
    force_due(&mut app);
    let mut out = Vec::new();
    poll(&mut app, &mut out).unwrap();
    assert!(app.recovery.task.is_some());

    super::super::save::do_atomic_save(&mut app, &mut out).unwrap();

    assert_eq!(std::fs::read_to_string(&original).unwrap(), "xbase");
    assert!(!crate::file::recovery::catnap_path(&original).exists());
    cleanup(&original);
}
