//! Purpose: verify Project services are absent in Plain and bracketed by explicit mode commands.
//! Owns: lifecycle, root selection, capability, and status-label tests for :project/:plain.
//! Must not: scan directories, run external commands, launch a terminal, or mutate content.
//! Invariants: Plain startup/descent has no ProjectSession; opt-in root is the file directory.
//! Phase: 5-b Project tooling bouncer tests.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::App;

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn run_command(app: &mut App, command: &str) {
    let mut out = Vec::new();
    app.handle_key_with(
        &mut out,
        key(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
    )
    .unwrap();
    for ch in command.chars() {
        app.handle_key_with(&mut out, key(KeyCode::Char(ch), KeyModifiers::NONE))
            .unwrap();
    }
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
}

#[test]
fn plain_start_constructs_no_project_session() {
    let app = App::new(None).unwrap();

    assert!(app.mode.is_plain());
    assert!(app.caps.is_plain_safe());
    assert!(app.project.is_none());
}

#[test]
fn project_command_constructs_session_at_active_file_directory() {
    let mut app = App::new(None).unwrap();
    app.file.path = Some(PathBuf::from("/tmp/catomic-project/src/main.rs"));

    run_command(&mut app, "project");

    assert!(app.mode.is_project());
    assert!(app.caps.linters && app.caps.repo_scan);
    assert_eq!(
        app.project.as_ref().unwrap().root(),
        PathBuf::from("/tmp/catomic-project/src")
    );
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("Project mode"));
    app.message = None;
    let mut out = Vec::new();
    app.render(&mut out).unwrap();
    assert!(String::from_utf8(out).unwrap().contains("project"));
}

#[test]
fn plain_command_drops_project_session_without_mode_chrome() {
    let mut app = App::new(None).unwrap();
    run_command(&mut app, "project");
    assert!(app.project.is_some());

    run_command(&mut app, "plain");

    assert!(app.mode.is_plain());
    assert!(app.caps.is_plain_safe());
    assert!(app.project.is_none());
    app.message = None;
    let mut out = Vec::new();
    app.render(&mut out).unwrap();
    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("[untitled]"));
    assert!(!rendered.contains("plain"));
}
