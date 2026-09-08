//! Purpose: prove explicit command execution, preview, stale guards, and undo integration.
//! Owns: App-level `:run`-path tests using local deterministic shell commands.
//! Must not: use network, write user files, depend on terminal setup, or skip confirmation.
//! Invariants: output never mutates before Enter; failed/stale output never applies.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

fn temp_path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "catomic_external_command_{name}_{}_{nonce}.txt",
        std::process::id()
    ))
}

#[test]
fn startup_constructs_no_command_task_or_preview() {
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

#[test]
fn undo_redo_to_the_same_text_while_command_runs_blocks_later_apply() {
    let mut app = super::super::App::new(None).unwrap();
    configure(
        &mut app,
        "[commands.slow]\ncommand = \"sleep 0.05; printf X\"\noutput = \"insert\"\n",
    );
    let mut out = Vec::new();
    app.handle_key_with(&mut out, key(KeyCode::Char('a'), KeyModifiers::NONE))
        .unwrap();

    start(&mut app, &mut out, "slow").unwrap();
    app.buffer.undo();
    app.buffer.redo();
    assert_eq!(app.buffer.to_string(), "a");

    wait_for_preview(&mut app, &mut out);
    handle_key(&mut app, &mut out, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();

    assert_eq!(app.buffer.to_string(), "a");
    assert!(app.message.as_deref().unwrap().contains("Source changed"));
}

#[test]
fn same_path_reload_that_resets_buffer_revision_blocks_later_apply() {
    let path = temp_path("reload_stale_guard");
    std::fs::write(&path, "before").unwrap();
    let source_path = path.to_string_lossy().into_owned();
    let mut app = super::super::App::new(Some(&source_path)).unwrap();
    configure(
        &mut app,
        "[commands.slow]\ncommand = \"sleep 0.05; printf X\"\noutput = \"insert\"\n",
    );
    let mut out = Vec::new();
    let source_revision = app.buffer.content_revision();
    let source_generation = app.file.content_generation;

    start(&mut app, &mut out, "slow").unwrap();
    std::fs::write(&path, "after").unwrap();
    let observation =
        crate::file::io::observe_external_file(Some(&path), app.file.disk_snapshot.as_ref());
    assert_eq!(
        observation.status,
        crate::file::io::ExternalFileStatus::Modified
    );
    super::super::reload::perform_observed_reload(&mut app, &observation);
    assert_eq!(app.buffer.to_string(), "after");
    assert_eq!(app.buffer.content_revision(), source_revision);
    assert_ne!(app.file.content_generation, source_generation);

    wait_for_preview(&mut app, &mut out);
    handle_key(&mut app, &mut out, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();

    assert_eq!(app.buffer.to_string(), "after");
    assert!(app.message.as_deref().unwrap().contains("Source changed"));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn no_input_insert_preparation_does_not_retain_a_large_source_snapshot() {
    const SOURCE_BYTES: usize = 64 * 1024 * 1024;
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(crate::buffer::PieceTable::from_owned_text(
        "a".repeat(SOURCE_BYTES),
    ));
    configure(
        &mut app,
        "[commands.insert]\ncommand = \"printf x\"\noutput = \"insert\"\n",
    );

    let retained_before = 0;
    let (_, sample) = crate::tests::perf::measure_live_allocations(|| {
        start(&mut app, &mut Vec::new(), "insert").unwrap();
    });
    let retained_after = sample.retained_bytes;
    let running = app.external_command.running.as_ref().unwrap();

    assert_eq!(retained_before, 0);
    assert!(
        retained_after.saturating_sub(retained_before) < 64 * 1024,
        "preparation retained {retained_after} bytes for a {SOURCE_BYTES}-byte source; \
         peak was {} bytes across {} allocations",
        sample.peak_bytes,
        sample.allocations,
    );
    assert!(
        sample.peak_bytes < 64 * 1024,
        "preparation peaked at {} bytes for a {SOURCE_BYTES}-byte source",
        sample.peak_bytes,
    );
    assert_eq!(running.source_revision, Some(app.buffer.content_revision()));
    assert!(cancel_all(&mut app));
}
