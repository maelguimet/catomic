//! Purpose: verify on-demand lint guards, polling, list view, and diagnostic navigation.
//! Owns: Phase 5-c App-level linter behavior tests without a real terminal.
//! Must not: load user config, auto-run tools, scan projects, mutate disk, or network.
//! Invariants: Plain/dirty spawn nothing; views never edit; jumps use 1-based diagnostics.
//! Phase: 5-c linter integration tests.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Cursor, PieceTable};
use crate::config::linters;
use crate::project::diagnostics::parse_common_output;

use super::super::{project_mode, App};

#[test]
fn plain_and_dirty_buffers_spawn_no_linter() {
    let config = linters::parse("[linters]\nrs = \"true {file}\"\n").unwrap();
    let mut app = App::new(None).unwrap();
    app.file.path = Some(PathBuf::from("/tmp/sample.rs"));
    let mut out = Vec::new();

    super::start_with_config(&mut app, &mut out, config.clone()).unwrap();
    assert!(app.project.is_none());
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("Project mode"));

    project_mode::switch_to_project(&mut app, &mut out).unwrap();
    app.file.dirty = true;
    super::start_with_config(&mut app, &mut out, config).unwrap();
    assert!(!app.project.as_ref().unwrap().is_linter_running());
    assert!(app.message.as_deref().unwrap_or("").contains("Save"));
}

#[test]
fn configured_linter_completes_into_project_diagnostics() {
    let config =
        linters::parse("[linters]\nrs = \"printf '%s:2:3: warning: found\\n' {file}\"\n").unwrap();
    let mut app = App::new(None).unwrap();
    app.file.path = Some(PathBuf::from("/tmp/sample.rs"));
    let mut out = Vec::new();
    project_mode::switch_to_project(&mut app, &mut out).unwrap();

    super::start_with_config(&mut app, &mut out, config).unwrap();
    assert!(app.project.as_ref().unwrap().is_linter_running());
    assert!(app.message.as_deref().unwrap_or("").contains("Running"));
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.project.as_ref().unwrap().is_linter_running() {
        super::poll(&mut app, &mut out).unwrap();
        assert!(Instant::now() < deadline, "linter integration timed out");
        std::thread::sleep(Duration::from_millis(5));
    }

    let diagnostics = app.project.as_ref().unwrap().diagnostics();
    assert_eq!(diagnostics.items.len(), 1);
    assert_eq!(
        (diagnostics.items[0].line, diagnostics.items[0].col),
        (2, 3)
    );
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("1 diagnostic"));
}

fn app_with_diagnostics() -> App {
    let mut app = App::new(None).unwrap();
    app.file.path = Some(PathBuf::from("/tmp/sample.rs"));
    app.buffer = Box::new(PieceTable::from_text("zero\none\ntwo"));
    let mut out = Vec::new();
    project_mode::switch_to_project(&mut app, &mut out).unwrap();
    let diagnostics = parse_common_output(
        "/tmp/sample.rs:2:3: warning: first\n/tmp/sample.rs:3:1: error: second\n",
        std::path::Path::new("/tmp"),
    );
    app.project.as_mut().unwrap().set_diagnostics(diagnostics);
    app
}

#[test]
fn diagnostics_view_is_read_only_and_escape_restores_source() {
    let mut app = app_with_diagnostics();
    let source = app.buffer.to_string();
    let mut out = Vec::new();

    super::show_diagnostics(&mut app, &mut out).unwrap();
    assert!(app.lint_view.is_some());
    assert!(String::from_utf8_lossy(&out).contains("warning"));
    app.handle_key_with(
        &mut out,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
    )
    .unwrap();
    assert_eq!(app.buffer.to_string(), source);
    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(app.lint_view.is_none());
}

#[test]
fn next_and_previous_diagnostics_jump_with_scalar_coordinates() {
    let mut app = app_with_diagnostics();
    let mut out = Vec::new();

    super::move_diagnostic(&mut app, &mut out, true).unwrap();
    assert_eq!(app.buffer.cursor(), Cursor { row: 1, col: 2 });
    super::move_diagnostic(&mut app, &mut out, true).unwrap();
    assert_eq!(app.buffer.cursor(), Cursor { row: 2, col: 0 });
    super::move_diagnostic(&mut app, &mut out, false).unwrap();
    assert_eq!(app.buffer.cursor(), Cursor { row: 1, col: 2 });
}

#[test]
fn cross_file_diagnostic_opens_a_buffer_and_jumps() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "catomic-cross-diagnostic-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&root).unwrap();
    let active = root.join("active.rs");
    let target = root.join("target.rs");
    std::fs::write(&active, "active\n").unwrap();
    std::fs::write(&target, "zero\nβeta\n").unwrap();
    let mut app = App::new(active.to_str()).unwrap();
    let mut out = Vec::new();
    project_mode::switch_to_project(&mut app, &mut out).unwrap();
    let diagnostics = parse_common_output(
        &format!("{}:2:2: error: cross file\n", target.display()),
        &root,
    );
    app.project.as_mut().unwrap().set_diagnostics(diagnostics);

    super::move_diagnostic(&mut app, &mut out, true).unwrap();

    assert_eq!(app.file.path.as_deref(), Some(target.as_path()));
    assert_eq!(app.buffer.cursor(), Cursor { row: 1, col: 1 });
    assert_eq!(app.buffer_count(), 2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn escape_cancels_a_running_linter() {
    let config = linters::parse("[linters]\nrs = \"while :; do :; done # {file}\"\n").unwrap();
    let mut app = App::new(None).unwrap();
    app.file.path = Some(PathBuf::from("/tmp/sample.rs"));
    let mut out = Vec::new();
    project_mode::switch_to_project(&mut app, &mut out).unwrap();
    super::start_with_config(&mut app, &mut out, config).unwrap();
    assert!(app.project.as_ref().unwrap().is_linter_running());

    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert!(!app.project.as_ref().unwrap().is_linter_running());
    assert!(app.message.as_deref().unwrap_or("").contains("cancelled"));
}
