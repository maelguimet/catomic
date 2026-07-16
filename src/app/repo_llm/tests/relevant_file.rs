//! Purpose: this file must prove the active repo file is always a disk-drift guard input.
//! Owns: loopback integration for untracked active-file changes hidden from Git status.
//! Must not: contact a live model, public endpoint, remote Git service, or user repository.
//! Invariants: changed active-file bytes discard output even when porcelain stays unchanged.
//! Phase: 6 acceptance hardening.

use std::fs;
use std::net::TcpListener;

use crossterm::event::KeyCode;

use super::*;

const DRAFT_PATCH: &str = "--- a/draft.txt\n+++ b/draft.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n";

#[test]
fn untracked_active_file_drift_before_confirmation_cancels_without_connecting() {
    let repo = TempRepo::new();
    let (path, mut app) = draft_app(&repo);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let settings = settings(format!("http://{}/v1", listener.local_addr().unwrap()));
    let mut out = Vec::new();
    begin_with_settings(&mut app, &mut out, "uppercase second line", settings).unwrap();
    poll_until_pending(&mut app, &mut out);

    fs::write(path, "changed outside\n").unwrap();
    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
    assert!(matches!(
        app.repo_llm_state.as_ref(),
        Some(RepoLlmState::CheckingSend(_))
    ));
    poll_until_send_checked(&mut app, &mut out);

    assert!(app.repo_llm_state.is_none());
    assert!(listener.accept().is_err());
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("Repository changed before confirmation"));
}

#[test]
fn untracked_active_file_drift_while_model_works_discards_response() {
    let repo = TempRepo::new();
    let (path, mut app) = draft_app(&repo);
    let (settings, server) = response_server(DRAFT_PATCH);
    let original = app.buffer.to_string();
    let mut out = Vec::new();
    begin_with_settings(&mut app, &mut out, "uppercase second line", settings).unwrap();
    poll_until_pending(&mut app, &mut out);

    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
    poll_until_running(&mut app, &mut out);
    fs::write(&path, "changed outside\n").unwrap();
    poll_until_finished(&mut app, &mut out);
    server.join().unwrap();

    assert!(app.llm_preview.is_none());
    assert_eq!(app.buffer.to_string(), original);
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("Repository changed while repo model worked"));
}

#[test]
fn untracked_active_file_drift_after_preview_blocks_apply() {
    let repo = TempRepo::new();
    let (path, mut app) = draft_app(&repo);
    let (settings, server) = response_server(DRAFT_PATCH);
    let original = app.buffer.to_string();
    let mut out = Vec::new();
    begin_with_settings(&mut app, &mut out, "uppercase second line", settings).unwrap();
    poll_until_pending(&mut app, &mut out);
    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
    poll_until_finished(&mut app, &mut out);
    server.join().unwrap();
    assert!(app.llm_preview.is_some());

    fs::write(path, "changed outside\n").unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter)).unwrap();

    assert!(app.llm_preview.is_none());
    assert_eq!(app.buffer.to_string(), original);
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("Repository changed since the request"));
}

fn draft_app(repo: &TempRepo) -> (PathBuf, super::super::super::App) {
    let path = repo.0.join("draft.txt");
    fs::write(&path, "one\ntwo\n").unwrap();
    let app = project_app_at(&path);
    (path, app)
}
