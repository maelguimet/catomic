//! App-level read-only large-file mode tests.
//!
//! Purpose: verify read-only Buffer guards at the App key/save layer without
//!   allocating large fixtures.
//! Owns: attempted edit/save behavior for file-backed read-only buffers.
//! Must not: test Huge threshold policy with 100 MiB+ defaults; use live watcher.
//! Invariants: read-only attempts do not dirty the buffer or modify disk.
//! Phase: 2B limited Huge-file storage foundation.

use super::super::*;
use super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};
use std::fs;

fn temp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("catomic_large_ro_{}_{}", std::process::id(), name));
    p
}

fn cleanup(path: &std::path::Path) {
    let _ = fs::remove_file(path);
}

fn app_with_read_only_buffer(path: &std::path::Path) -> App {
    let mut app = App::new(None).unwrap();
    app.file.path = Some(path.to_path_buf());
    app.file.disk_snapshot = crate::file::io::capture_file_snapshot(path).ok();
    app.file.size_bytes = Some(crate::file::size::LARGE_FILE_LIMIT_BYTES + 1);
    app.file.size_tier = Some(crate::file::size::FileSizeTier::Huge);
    app.buffer = Box::new(crate::buffer::LargeFileBuffer::open(path).unwrap());
    app
}

#[test]
fn read_only_buffer_edit_attempt_sets_message_without_dirty_or_disk_write() {
    let path = temp_path("edit.txt");
    cleanup(&path);
    fs::write(&path, "first\nsecond").unwrap();
    let mut app = app_with_read_only_buffer(&path);
    let mut out = Vec::new();

    app.handle_key_with(&mut out, make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    assert!(!app.file.dirty);
    assert_eq!(fs::read_to_string(&path).unwrap(), "first\nsecond");
    assert_eq!(app.buffer.line(0).as_deref(), Some("first"));
    assert!(app.message.as_deref().unwrap_or("").contains("read-only"));
    assert!(String::from_utf8_lossy(&out).contains("read-only"));

    cleanup(&path);
}

#[test]
fn read_only_buffer_save_is_disabled_and_preserves_disk() {
    let path = temp_path("save.txt");
    cleanup(&path);
    fs::write(&path, "stable\ncontent").unwrap();
    let mut app = app_with_read_only_buffer(&path);
    let mut out = Vec::new();

    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert!(!app.file.dirty);
    assert_eq!(fs::read_to_string(&path).unwrap(), "stable\ncontent");
    assert!(app.pending_save_conflict.is_none());
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("save disabled"));
    assert!(String::from_utf8_lossy(&out).contains("save disabled"));

    cleanup(&path);
}
