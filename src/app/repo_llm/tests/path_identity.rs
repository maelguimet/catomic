//! Purpose: this file must prove repo LLM requests remain bound to one active file path.
//! Owns: deterministic path-drift tests across preparation, confirmation, and response.
//! Must not: contact a live model, public endpoint, remote Git service, or user repository.
//! Invariants: path drift cancels or discards before any proposal can reach preview.

use std::net::TcpListener;

use crossterm::event::KeyCode;

use super::*;

#[test]
fn path_change_while_repo_context_is_built_cancels_request() {
    let repo = TempRepo::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let settings = settings(format!("http://{}/v1", listener.local_addr().unwrap()));
    let mut app = project_app(&repo);
    let mut out = Vec::new();

    begin_with_settings(&mut app, &mut out, "write tests", settings).unwrap();
    app.file.path = Some(repo.0.join("other.txt"));
    poll_until_pending(&mut app, &mut out);

    assert!(app.repo_llm_state.is_none());
    assert!(listener.accept().is_err());
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("path changed while repo context was built"));
}

#[test]
fn path_change_before_confirmation_cancels_without_connecting() {
    let repo = TempRepo::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let settings = settings(format!("http://{}/v1", listener.local_addr().unwrap()));
    let mut app = project_app(&repo);
    let mut out = Vec::new();
    begin_with_settings(&mut app, &mut out, "write tests", settings).unwrap();
    poll_until_pending(&mut app, &mut out);

    app.file.path = Some(repo.0.join("other.txt"));
    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();

    assert!(app.repo_llm_state.is_none());
    assert!(listener.accept().is_err());
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("path changed before confirmation"));
}

#[test]
fn path_change_while_repo_model_works_discards_response() {
    let repo = TempRepo::new();
    let (settings, server) = patch_server();
    let mut app = project_app(&repo);
    let original = app.buffer.to_string();
    let mut out = Vec::new();
    begin_with_settings(&mut app, &mut out, "uppercase second line", settings).unwrap();
    poll_until_pending(&mut app, &mut out);

    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
    poll_until_running(&mut app, &mut out);
    app.file.path = Some(repo.0.join("other.txt"));
    poll_until_finished(&mut app, &mut out);
    server.join().unwrap();

    assert!(app.surfaces.llm_preview.is_none());
    assert_eq!(app.buffer.to_string(), original);
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("path changed while repo model worked"));
}
