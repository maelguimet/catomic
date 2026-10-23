//! Focused App/FileState size metadata tests.
//!
//! Purpose: verify size_bytes + size_tier are captured from metadata only at the
//!   documented points (open existing, open missing, successful save, confirmed
//!   reload Modified, confirmed reload Deleted) and left unchanged on failure.
//! Owns: the minimal required size-metadata cases for App::new / save / reload.
//! Must not: allocate huge files; change save/reload/watch behavior or messages;
//!   assert on UI strings beyond size bookkeeping; depend on live watcher.
//! Invariants: None for no-path and for missing/deleted; Present len+tier for
//!   real on-disk files after open/save/reload-Modified; no content-derived sizes.

use super::super::*;
use super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};
use std::fs;

fn temp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("catomic_fsize_{}_{}", std::process::id(), name));
    p
}

fn cleanup(p: &std::path::Path) {
    let _ = fs::remove_file(p);
}

#[test]
fn app_new_none_has_no_size_metadata() {
    let app = App::new(None).unwrap();
    assert!(app.file.path.is_none());
    assert!(app.file.size_bytes.is_none());
    assert!(app.file.size_tier.is_none());
}

#[test]
fn app_new_existing_records_size_bytes_and_tier() {
    let p = temp_path("exist_size.txt");
    cleanup(&p);
    let data = "abc\ndef\n";
    fs::write(&p, data).unwrap();

    let app = App::new(Some(&p.to_string_lossy())).unwrap();
    assert!(app.file.path.is_some());
    assert_eq!(app.file.size_bytes, Some(8));
    assert_eq!(
        app.file.size_tier,
        Some(crate::file::size::FileSizeTier::Small)
    );
    assert_eq!(app.buffer.to_string(), data);

    cleanup(&p);
}

#[test]
fn app_new_missing_has_empty_buffer_and_size_none() {
    let p = temp_path("missing_for_new_size_zzz.txt");
    let _ = fs::remove_file(&p);

    let app = App::new(Some(&p.to_string_lossy())).unwrap();
    assert!(app.file.path.is_some());
    assert_eq!(app.buffer.to_string(), "");
    assert!(app.file.size_bytes.is_none());
    assert!(app.file.size_tier.is_none());
    assert_eq!(
        app.file.disk_snapshot,
        Some(crate::file::io::FileSnapshot::Absent)
    );
}

#[test]
fn successful_save_with_explicit_first_path_updates_size_metadata() {
    let p = temp_path("first_save_explicit.txt");
    cleanup(&p);
    let _ = fs::remove_file(&p);

    let mut app = App::new(None).unwrap();
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    assert!(app.file.size_bytes.is_none());
    assert!(app.file.path.is_none());

    app.file.path = Some(p.clone());
    app.file.disk_snapshot = Some(crate::file::io::FileSnapshot::Absent);

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(!app.file.dirty, "save to explicit path must clear dirty");
    assert!(app.file.path.as_ref() == Some(&p));
    let got = app
        .file
        .size_bytes
        .expect("size must be recorded after successful save to explicit path");
    let buf_len = app.buffer.to_string().len() as u64;
    assert_eq!(got, buf_len, "size must match written content len");
    assert_eq!(
        app.file.size_tier,
        Some(crate::file::size::FileSizeTier::Small)
    );
    assert!(p.exists(), "target must have been created");

    cleanup(&p);
}

#[test]
fn successful_save_existing_updates_size_after_content_change() {
    let p = temp_path("save_updates_size.txt");
    cleanup(&p);
    fs::write(&p, "OLD").unwrap();

    let mut app = App::new(Some(&p.to_string_lossy())).unwrap();
    assert_eq!(app.file.size_bytes, Some(3));

    app.handle_key(make_key(KeyCode::Char('a'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('b'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('c'), KeyModifiers::NONE))
        .unwrap();

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(!app.file.dirty);
    assert_eq!(app.file.size_bytes, Some(6));
    assert_eq!(
        app.file.size_tier,
        Some(crate::file::size::FileSizeTier::Small)
    );

    cleanup(&p);
}

#[test]
fn confirmed_reload_modified_updates_size_to_external() {
    let p = temp_path("reload_mod_size.txt");
    cleanup(&p);
    fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p.to_string_lossy())).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    fs::write(&p, "EXTERNALLONGER").unwrap();

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_reload.is_some());

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "EXTERNALLONGER");
    assert!(!app.file.dirty);
    assert_eq!(app.file.size_bytes, Some(14));
    assert_eq!(
        app.file.size_tier,
        Some(crate::file::size::FileSizeTier::Small)
    );

    cleanup(&p);
}

#[test]
fn confirmed_reload_deleted_clears_size_metadata() {
    let p = temp_path("reload_del_size.txt");
    cleanup(&p);
    fs::write(&p, "TOGO").unwrap();

    let mut app = App::new(Some(&p.to_string_lossy())).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.file.size_bytes, Some(4));

    let _ = fs::remove_file(&p);

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_reload.is_some());

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "");
    assert!(!app.file.dirty);
    assert_eq!(
        app.file.disk_snapshot,
        Some(crate::file::io::FileSnapshot::Absent)
    );
    assert!(app.file.size_bytes.is_none());
    assert!(app.file.size_tier.is_none());

    fs::write(&p, "TOGO").unwrap();
    let _ = fs::remove_file(&p);
}

#[test]
fn failed_save_does_not_update_size_metadata() {
    let bad = {
        let mut d = std::env::temp_dir();
        d.push(format!("catomic_fsize_bad_dir_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    };
    assert!(bad.is_dir());

    let mut app = App::new(None).unwrap();
    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::NONE))
        .unwrap();
    app.file.path = Some(bad.clone());

    let before_size = app.file.size_bytes;
    let before_tier = app.file.size_tier;

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(app.file.dirty);
    assert!(app.message.as_deref().unwrap_or("").contains("Save error"));
    assert_eq!(app.message_role, crate::terminal::render::StatusRole::Error);
    assert_eq!(app.file.size_bytes, before_size);
    assert_eq!(app.file.size_tier, before_tier);

    let _ = fs::remove_dir_all(&bad);
}

#[test]
fn size_metadata_does_not_alter_snapshot_or_conflict_behavior_smoke() {
    let p = temp_path("size_no_side_on_snapshot.txt");
    cleanup(&p);
    fs::write(&p, "KEEP").unwrap();

    let mut app = App::new(Some(&p.to_string_lossy())).unwrap();
    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::NONE))
        .unwrap();
    fs::write(&p, "CHANGED").unwrap();

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(app.pending_save_conflict.is_some());
    assert_eq!(
        app.message_role,
        crate::terminal::render::StatusRole::Warning
    );
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("changed on disk"));

    cleanup(&p);
}
