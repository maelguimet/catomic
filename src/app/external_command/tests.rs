//! Purpose: prove explicit command execution, preview, stale guards, and undo integration.
//! Owns: App-level `:run`-path tests using local deterministic shell commands.
//! Must not: use network, write user files, depend on terminal setup, or skip confirmation.
//! Invariants: output never mutates before Enter; failed/stale output never applies.
//! Phase: 7 external command acceptance.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;

fn configure(app: &mut super::super::App, body: &str) {
    app.command_config = crate::config::commands::parse(body).unwrap();
}

fn wait_for_preview(app: &mut super::super::App, out: &mut Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !is_viewing(app) {
        poll(app, out).unwrap();
        assert!(Instant::now() < deadline, "command preview timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn plain_start_constructs_no_command_task_or_preview() {
    let app = super::super::App::new(None).unwrap();

    assert!(app.external_command.running.is_none());
    assert!(app.external_command.preview.is_none());
}

#[test]
fn unknown_command_sets_error_role_at_the_emission_boundary() {
    let mut app = super::super::App::new(None).unwrap();

    start(&mut app, &mut Vec::new(), "missing").unwrap();

    assert_eq!(app.message_role, crate::terminal::render::StatusRole::Error);
}

#[test]
fn subprocess_failure_sets_error_role_at_the_emission_boundary() {
    let mut app = super::super::App::new(None).unwrap();

    finish_error(&mut app, &mut Vec::new(), "fixture", "failed: boom").unwrap();

    assert_eq!(app.message_role, crate::terminal::render::StatusRole::Error);
}

#[test]
fn insert_output_is_previewed_then_applied_as_one_undoable_edit() {
    let mut app = super::super::App::new(None).unwrap();
    configure(
        &mut app,
        "[commands.word]\ncommand = \"printf CAT\"\noutput = \"insert\"\n",
    );
    let mut out = Vec::new();

    start(&mut app, &mut out, "word").unwrap();
    assert_eq!(app.buffer.to_string(), "");
    wait_for_preview(&mut app, &mut out);
    assert_eq!(app.buffer.to_string(), "");

    handle_key(&mut app, &mut out, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();
    assert_eq!(app.buffer.to_string(), "CAT");
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "");
}

#[test]
fn selected_input_can_be_replaced_after_preview() {
    let mut app = super::super::App::new(None).unwrap();
    for ch in "cat".chars() {
        app.buffer.insert_char(ch);
    }
    app.handle_key_with(
        &mut Vec::new(),
        key(KeyCode::Char('a'), KeyModifiers::CONTROL),
    )
    .unwrap();
    configure(
        &mut app,
        "[commands.upper]\ncommand = \"tr a-z A-Z\"\ninput = \"selection\"\n\
         output = \"replace-input\"\n",
    );
    let mut out = Vec::new();

    start(&mut app, &mut out, "upper").unwrap();
    wait_for_preview(&mut app, &mut out);
    handle_key(&mut app, &mut out, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();

    assert_eq!(app.buffer.to_string(), "CAT");
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "cat");
}

#[test]
fn failed_command_output_is_read_only_and_cannot_apply() {
    let mut app = super::super::App::new(None).unwrap();
    configure(
        &mut app,
        "[commands.fail]\ncommand = \"printf bad; exit 7\"\noutput = \"insert\"\n",
    );
    let mut out = Vec::new();

    start(&mut app, &mut out, "fail").unwrap();
    wait_for_preview(&mut app, &mut out);
    assert!(app
        .external_command
        .preview
        .as_ref()
        .unwrap()
        .target
        .is_none());
    handle_key(&mut app, &mut out, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();

    assert_eq!(app.buffer.to_string(), "");
}

#[test]
fn source_edit_while_command_runs_blocks_later_apply() {
    let mut app = super::super::App::new(None).unwrap();
    configure(
        &mut app,
        "[commands.slow]\ncommand = \"sleep 0.05; printf X\"\noutput = \"insert\"\n",
    );
    let mut out = Vec::new();

    start(&mut app, &mut out, "slow").unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Char('a'), KeyModifiers::NONE))
        .unwrap();
    wait_for_preview(&mut app, &mut out);
    handle_key(&mut app, &mut out, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();

    assert_eq!(app.buffer.to_string(), "a");
    assert!(app.message.as_deref().unwrap().contains("Source changed"));
}
