//! Purpose: this file must prove explicit current-buffer invocation and confirmation gating.
//! Owns: selection/block drafts, no-network-before-Enter, loopback completion, and preview.
//! Must not: contact a live model, public endpoint, user service, or external network.
//! Invariants: Plain startup has no request objects; model output never bypasses preview.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::buffer::{Cursor, PieceTable};
use crate::llm::context::ContextScope;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;

static NEXT_COMMAND_FIXTURE: AtomicUsize = AtomicUsize::new(0);

#[test]
fn plain_start_constructs_no_pending_request_task_or_client() {
    let app = super::super::App::new(None).unwrap();
    assert!(app.pending_llm_request.is_none());
    assert!(app.llm_task.is_none());
    assert!(!app.caps.network_llm);
}

#[test]
fn first_meow_builds_confirmation_without_connecting() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let settings = settings(format!("http://{}/v1", listener.local_addr().unwrap()));
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("alpha beta"));
    app.buffer.set_cursor(Cursor { row: 0, col: 0 });
    let mut out = Vec::new();
    app.handle_key_with(&mut out, key(KeyCode::Right, KeyModifiers::SHIFT))
        .unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Right, KeyModifiers::SHIFT))
        .unwrap();

    begin_with_settings(
        &mut app,
        &mut out,
        CurrentLlmCommand::Meow,
        "refactor",
        settings,
    )
    .unwrap();

    assert!(app.pending_llm_request.is_some());
    assert!(app.llm_task.is_none());
    assert!(
        listener.accept().is_err(),
        "first invocation must not connect"
    );
    assert!(app.message.as_deref().unwrap().contains("Enter confirms"));
    let confirmation = app.message.as_deref().unwrap();
    assert!(confirmation.starts_with("To http://127.0.0.1:"));
    assert!(confirmation.contains("preset local model test-model"));

    app.handle_key_with(&mut out, key(KeyCode::F(10), KeyModifiers::NONE))
        .unwrap();
    assert!(app.pending_llm_request.is_some());
    assert!(!super::super::model_picker::is_viewing(&app));
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("send not confirmed"));
}

#[test]
fn confirmed_loopback_response_enters_preview_then_applies_on_second_enter() {
    let (settings, server) = patch_server();
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("one\ntwo\n"));
    app.file.path = Some("note.txt".into());
    let mut out = Vec::new();
    begin_with_settings(
        &mut app,
        &mut out,
        CurrentLlmCommand::BigMeow,
        "refactor",
        settings,
    )
    .unwrap();
    assert!(app.pending_llm_request.is_some());
    assert!(app.llm_task.is_none());

    handle_key(&mut app, &mut out, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();
    assert!(app.pending_llm_request.is_none());
    assert!(app.llm_task.is_some());
    assert_eq!(app.buffer.to_string(), "one\ntwo\n");
    poll_until_done(&mut app, &mut out);
    server.join().unwrap();

    assert!(
        app.surfaces.llm_preview.is_some(),
        "status: {:?}",
        app.message
    );
    assert_eq!(app.buffer.to_string(), "one\ntwo\n");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.buffer.to_string(), "one\nTWO\n");
}

#[test]
fn current_file_patch_for_another_path_is_refused_before_preview() {
    let patch = "--- a/other.txt\n+++ b/other.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n";
    let (settings, server) = response_server(patch);
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("one\ntwo\n"));
    app.file.path = Some("note.txt".into());
    let original = app.buffer.to_string();
    let mut out = Vec::new();
    begin_with_settings(
        &mut app,
        &mut out,
        CurrentLlmCommand::BigMeow,
        "refactor",
        settings,
    )
    .unwrap();

    handle_key(&mut app, &mut out, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();
    poll_until_done(&mut app, &mut out);
    server.join().unwrap();

    assert!(app.surfaces.llm_preview.is_none());
    assert_eq!(app.buffer.to_string(), original);
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("Invalid LLM patch"));
}

#[test]
fn path_change_before_confirmation_cancels_without_connecting() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let settings = settings(format!("http://{}/v1", listener.local_addr().unwrap()));
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("one\ntwo\n"));
    let mut out = Vec::new();
    begin_with_settings(
        &mut app,
        &mut out,
        CurrentLlmCommand::BigMeow,
        "refactor",
        settings,
    )
    .unwrap();

    app.file.path = Some("saved-later.txt".into());
    handle_key(&mut app, &mut out, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();

    assert!(app.pending_llm_request.is_none());
    assert!(app.llm_task.is_none());
    assert!(listener.accept().is_err());
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("path changed before confirmation"));
}

#[test]
fn path_change_while_model_works_discards_response() {
    let (settings, server) = patch_server();
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("one\ntwo\n"));
    app.file.path = Some("note.txt".into());
    let original = app.buffer.to_string();
    let mut out = Vec::new();
    begin_with_settings(
        &mut app,
        &mut out,
        CurrentLlmCommand::BigMeow,
        "refactor",
        settings,
    )
    .unwrap();

    handle_key(&mut app, &mut out, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();
    app.file.path = Some("renamed.txt".into());
    poll_until_done(&mut app, &mut out);
    server.join().unwrap();

    assert!(app.surfaces.llm_preview.is_none());
    assert_eq!(app.buffer.to_string(), original);
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("path changed while the model was working"));
}

#[test]
fn confirmed_marked_region_response_previews_then_replaces_only_selection() {
    let (settings, server) = response_server(r#"{"catomic_replacement":"TWO"}"#);
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("one\ntwo\nthree\n"));
    app.buffer.set_cursor(Cursor { row: 1, col: 0 });
    let mut out = Vec::new();
    for _ in 0..3 {
        app.handle_key_with(&mut out, key(KeyCode::Right, KeyModifiers::SHIFT))
            .unwrap();
    }
    begin_with_settings(
        &mut app,
        &mut out,
        CurrentLlmCommand::Meow,
        "uppercase",
        settings,
    )
    .unwrap();

    handle_key(&mut app, &mut out, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();
    poll_until_done(&mut app, &mut out);
    server.join().unwrap();
    assert!(
        app.surfaces.llm_preview.is_some(),
        "status: {:?}",
        app.message
    );
    assert_eq!(app.buffer.to_string(), "one\ntwo\nthree\n");

    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.buffer.to_string(), "one\nTWO\nthree\n");
}

#[test]
fn confirmed_explain_response_opens_read_only_answer_instead_of_edit_preview() {
    let (settings, server) = response_server("This function returns its input.");
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("fn identity(x: i32) -> i32 { x }"));
    let history = app.buffer.edit_history_position();
    let mut out = Vec::new();
    begin_with_settings(
        &mut app,
        &mut out,
        CurrentLlmCommand::BigMeow,
        "Explain this function.",
        settings,
    )
    .unwrap();

    assert_eq!(
        app.pending_llm_request.as_ref().unwrap().purpose,
        RequestPurpose::Explain
    );
    handle_key(&mut app, &mut out, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();
    poll_until_done(&mut app, &mut out);
    server.join().unwrap();

    assert!(app.surfaces.llm_answer.is_some());
    assert!(app.surfaces.llm_preview.is_none());
    assert_eq!(app.buffer.to_string(), "fn identity(x: i32) -> i32 { x }");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.buffer.edit_history_position(), history);
}

#[test]
fn confirmed_command_backend_uses_the_same_preview_apply_undo_and_no_save_path() {
    let suffix = NEXT_COMMAND_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "catomic-app-command-llm-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let program = root.join("fake claude");
    let marker = root.join("started marker");
    let source_path = root.join("note.txt");
    fs::write(&source_path, "one\ntwo\n").unwrap();
    let patch = "--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n";
    let response = serde_json::json!({
        "type": "result",
        "is_error": false,
        "result": patch,
    });
    fs::write(
        &program,
        format!(
            "#!/bin/sh\ninput=$(cat)\ncase \"$input\" in *\"$2\"*) exit 9 ;; esac\nprintf ran > \"$1\"\nprintf '%s' '{}'\n",
            response
        ),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    let preset = crate::config::llm::parse(&format!(
        "[[llm.backends]]\nname='local command'\ntype='command'\nprogram={:?}\nargs=[{:?},{:?}]\nmodel='fixture'\noutput='claude-json-v1'\ntimeout_secs=2\n",
        program.to_string_lossy(), marker.to_string_lossy(), source_path.to_string_lossy()
    ))
    .unwrap()
    .default_preset()
    .clone();
    let mut app = super::super::App::new(source_path.to_str()).unwrap();
    let mut out = Vec::new();

    begin_with_settings(
        &mut app,
        &mut out,
        CurrentLlmCommand::BigMeow,
        "uppercase second line",
        preset,
    )
    .unwrap();
    assert!(
        !marker.exists(),
        "command must not start before confirmation"
    );
    assert!(app.message.as_deref().unwrap().contains("local command"));

    handle_key(&mut app, &mut out, key(KeyCode::Enter, KeyModifiers::NONE)).unwrap();
    poll_until_done(&mut app, &mut out);
    assert!(marker.exists());
    assert!(
        app.surfaces.llm_preview.is_some(),
        "status: {:?}",
        app.message
    );
    assert_eq!(fs::read_to_string(&source_path).unwrap(), "one\ntwo\n");

    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.buffer.to_string(), "one\nTWO\n");
    assert!(app.file.dirty);
    assert_eq!(fs::read_to_string(&source_path).unwrap(), "one\ntwo\n");
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "one\ntwo\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn meow_without_selection_uses_instruction_block_at_cursor() {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text(
        "before\n>>> catomic\nWrite tests.\n<<<\nafter",
    ));
    app.buffer.set_cursor(Cursor { row: 2, col: 0 });
    let mut out = Vec::new();

    begin_with_settings(
        &mut app,
        &mut out,
        CurrentLlmCommand::Meow,
        "",
        settings("http://127.0.0.1:9/v1".to_string()),
    )
    .unwrap();

    let pending = app.pending_llm_request.as_ref().unwrap();
    assert_eq!(pending.draft.instruction, "Write tests.");
    assert_eq!(pending.draft.context.scope, ContextScope::InstructionBlock);
}

#[test]
fn prompt_names_the_path_instruction_extent_and_confirmed_sensitivity() {
    let draft = context::for_selection(
        "API_KEY=redacted",
        4,
        "write tests",
        Some(Path::new(".env")),
    )
    .unwrap();
    let prompt = prompt::user_prompt(&draft, ".env");

    assert!(prompt.contains("Path: .env"));
    assert!(prompt.contains("1-based line: 5"));
    assert!(prompt.contains("Instruction:\nwrite tests"));
    assert!(prompt.contains("dotfile"));
    assert!(prompt.contains("secret-like line 5"));
}

#[test]
fn only_an_explicit_explain_verb_selects_the_read_only_response_path() {
    let explain = context::for_current_file("code", "Explain: why", None).unwrap();
    let tests = context::for_current_file("code", "write tests", None).unwrap();
    assert_eq!(prompt::purpose(&explain), RequestPurpose::Explain);
    assert_eq!(prompt::purpose(&tests), RequestPurpose::Edit);
}

fn settings(base_url: String) -> BackendPreset {
    crate::config::llm::parse(&format!(
        "[llm]\nbase_url={base_url:?}\nmodel='test-model'\ntimeout_secs=2\n"
    ))
    .unwrap()
    .default_preset()
    .clone()
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn poll_until_done(app: &mut super::super::App, out: &mut Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.llm_task.is_some() {
        poll(app, out).unwrap();
        assert!(Instant::now() < deadline, "LLM integration timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn patch_server() -> (BackendPreset, std::thread::JoinHandle<()>) {
    let patch = "--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n";
    response_server(patch)
}

fn response_server(content: &str) -> (BackendPreset, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let content = content.to_string();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let count = stream.read(&mut chunk).unwrap();
            request.extend_from_slice(&chunk[..count]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let body = serde_json::json!({"choices":[{"message":{"content":content}}]}).to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    (settings(format!("http://{address}/v1")), server)
}
