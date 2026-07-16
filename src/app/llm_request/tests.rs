//! Purpose: this file must prove explicit current-buffer invocation and confirmation gating.
//! Owns: selection/block drafts, no-network-before-Enter, loopback completion, and preview.
//! Must not: contact a live model, public endpoint, user service, or external network.
//! Invariants: Plain startup has no request objects; model output never bypasses preview.
//! Phase: 6 (LLM, Powerful but Caged).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use crate::buffer::{Cursor, PieceTable};
use crate::llm::context::ContextScope;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;

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

    assert!(app.llm_preview.is_some());
    assert_eq!(app.buffer.to_string(), "one\ntwo\n");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.buffer.to_string(), "one\nTWO\n");
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

fn settings(base_url: String) -> LlmSettings {
    LlmSettings {
        base_url,
        model: "test-model".to_string(),
        api_key_env: "CATOMIC_TEST_MISSING_KEY".to_string(),
        timeout: Duration::from_secs(2),
    }
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

fn patch_server() -> (LlmSettings, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
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
        let patch = "--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n";
        let body = serde_json::json!({"choices":[{"message":{"content":patch}}]}).to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    (settings(format!("http://{address}/v1")), server)
}
