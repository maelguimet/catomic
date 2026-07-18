//! Purpose: this file must prove Project gating, no-network preparation, and guarded preview.
//! Owns: isolated Git repos and loopback-only repo command integration.
//! Must not: contact a live model, public endpoint, remote Git service, or user repository.
//! Invariants: Plain constructs no state; network starts only on Enter; repo drift blocks apply.
//! Phase: 6 (LLM Context Broker).

mod path_identity;
mod relevant_file;

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[test]
fn plain_command_constructs_no_repo_state() {
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();

    begin(&mut app, &mut out, RepoLlmCommand::GitMeow, "write tests").unwrap();

    assert!(app.repo_llm_state.is_none());
    assert!(app.project.is_none());
    assert!(app.message.as_deref().unwrap().contains("Project mode"));
}

#[test]
fn repo_command_variants_have_distinct_bounded_context_profiles() {
    assert_eq!(RepoLlmCommand::GitMeow.name(), "gitmeow");
    assert_eq!(RepoLlmCommand::GitMeow.profile(), "focused");
    assert_eq!(RepoLlmCommand::GitMeow.context_budget(), 64 * 1024);
    assert_eq!(RepoLlmCommand::MegaMeow.name(), "megameow");
    assert_eq!(RepoLlmCommand::MegaMeow.profile(), "broader");
    assert_eq!(RepoLlmCommand::MegaMeow.context_budget(), 128 * 1024);
}

#[test]
fn project_preparation_reaches_confirmation_without_connecting() {
    let repo = TempRepo::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let settings = settings(format!("http://{}/v1", listener.local_addr().unwrap()));
    let mut app = project_app(&repo);
    let mut out = Vec::new();

    begin_with_settings(&mut app, &mut out, "write tests", settings).unwrap();
    assert!(matches!(
        app.repo_llm_state.as_ref(),
        Some(RepoLlmState::Preparing(_))
    ));
    assert!(listener.accept().is_err());
    poll_until_pending(&mut app, &mut out);

    assert!(matches!(
        app.repo_llm_state.as_ref(),
        Some(RepoLlmState::Pending(_))
    ));
    let RepoLlmState::Pending(pending) = app.repo_llm_state.as_ref().unwrap() else {
        unreachable!()
    };
    assert_eq!(
        pending.prepared.broker.remaining_budget() + pending.prepared.initial_context.len(),
        64 * 1024
    );
    assert!(listener.accept().is_err());
    let message = app.message.as_deref().unwrap();
    assert!(message.contains("gitmeow focused context"));
    assert!(message.contains("at most 64 KiB"));
    assert!(message.contains("repo bytes"));
    assert!(message.contains("Enter confirms"));

    fs::write(repo.0.join("other.txt"), "changed before send\n").unwrap();
    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
    assert!(matches!(
        app.repo_llm_state.as_ref(),
        Some(RepoLlmState::CheckingSend(_))
    ));
    assert!(listener.accept().is_err());
    poll_until_send_checked(&mut app, &mut out);
    assert!(app.repo_llm_state.is_none());
    assert!(listener.accept().is_err());
}

#[test]
fn megameow_profile_survives_async_preparation_without_connecting() {
    let repo = TempRepo::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let settings = settings(format!("http://{}/v1", listener.local_addr().unwrap()));
    let mut app = project_app(&repo);
    let mut out = Vec::new();

    begin_with_command_and_settings(
        &mut app,
        &mut out,
        RepoLlmCommand::MegaMeow,
        "write tests",
        settings,
    )
    .unwrap();
    assert!(matches!(
        app.repo_llm_state.as_ref(),
        Some(RepoLlmState::Preparing(Preparing {
            command: RepoLlmCommand::MegaMeow,
            ..
        }))
    ));
    assert!(listener.accept().is_err());

    poll_until_pending(&mut app, &mut out);
    assert!(matches!(
        app.repo_llm_state.as_ref(),
        Some(RepoLlmState::Pending(pending))
            if pending.command == RepoLlmCommand::MegaMeow
    ));
    let RepoLlmState::Pending(pending) = app.repo_llm_state.as_ref().unwrap() else {
        unreachable!()
    };
    assert_eq!(
        pending.prepared.broker.remaining_budget() + pending.prepared.initial_context.len(),
        128 * 1024
    );
    let message = app.message.as_deref().unwrap();
    assert!(message.contains("megameow broader context"));
    assert!(message.contains("at most 128 KiB"));
    assert!(listener.accept().is_err());
}

#[test]
fn confirmed_patch_previews_but_repo_drift_blocks_apply() {
    let repo = TempRepo::new();
    let (settings, server) = patch_server();
    let mut app = project_app(&repo);
    let original = app.buffer.to_string();
    let mut out = Vec::new();
    begin_with_settings(&mut app, &mut out, "uppercase second line", settings).unwrap();
    poll_until_pending(&mut app, &mut out);

    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
    poll_until_finished(&mut app, &mut out);
    server.join().unwrap();
    assert!(app.llm_preview.is_some());
    assert_eq!(app.buffer.to_string(), original);

    fs::write(repo.0.join("other.txt"), "changed again\n").unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter)).unwrap();
    assert!(matches!(
        app.repo_llm_state.as_ref(),
        Some(RepoLlmState::CheckingApply(_))
    ));
    assert!(app.llm_preview.is_some());
    poll_until_finished(&mut app, &mut out);
    assert_eq!(app.buffer.to_string(), original);
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("Repository changed"));
}

#[test]
fn confirmed_repo_patch_checks_then_applies_as_one_undo_step() {
    let repo = TempRepo::new();
    let (settings, server) = patch_server();
    let mut app = project_app(&repo);
    let original = app.buffer.to_string();
    let original_position = app.buffer.edit_history_position();
    let mut out = Vec::new();
    begin_with_settings(&mut app, &mut out, "uppercase second line", settings).unwrap();
    poll_until_pending(&mut app, &mut out);
    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
    poll_until_finished(&mut app, &mut out);
    server.join().unwrap();

    app.handle_key_with(&mut out, key(KeyCode::Enter)).unwrap();
    assert!(matches!(
        app.repo_llm_state.as_ref(),
        Some(RepoLlmState::CheckingApply(_))
    ));
    assert_eq!(app.buffer.to_string(), original);
    poll_until_finished(&mut app, &mut out);

    assert_eq!(app.buffer.to_string(), "one\nTWO\n");
    assert!(app.llm_preview.is_none());
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), original);
    assert_eq!(app.buffer.edit_history_position(), original_position);
}

#[test]
fn confirmed_command_backend_reuses_repo_drift_preview_and_no_save_contract() {
    let repo = TempRepo::new();
    let suffix = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let command_root = std::env::temp_dir().join(format!(
        "catomic-repo-command-llm-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir(&command_root).unwrap();
    let program = command_root.join("fake codex adapter");
    let marker = command_root.join("started");
    let patch = "--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n";
    let events = [
        serde_json::json!({"type":"thread.started"}),
        serde_json::json!({"type":"turn.started"}),
        serde_json::json!({"type":"item.completed","item":{"type":"agent_message","text":patch}}),
        serde_json::json!({"type":"turn.completed"}),
    ]
    .map(|event| format!("'{}'", event))
    .join(" ");
    fs::write(
        &program,
        format!(
            "#!/bin/sh\ncat >/dev/null\nprintf ran > \"$1\"\nprintf '%s\\n' {}\n",
            events
        ),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    let preset = crate::config::llm::parse(&format!(
        "[[llm.backends]]\nname='repo command'\ntype='command'\nprogram={:?}\nargs=[{:?}]\nmodel='fixture'\noutput='codex-jsonl-v1'\ntimeout_secs=2\n",
        program.to_string_lossy(), marker.to_string_lossy()
    ))
    .unwrap()
    .default_preset()
    .clone();
    let mut app = project_app(&repo);
    let mut out = Vec::new();

    begin_with_settings(&mut app, &mut out, "uppercase second line", preset).unwrap();
    poll_until_pending(&mut app, &mut out);
    assert!(!marker.exists(), "repo preparation must not start command");
    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
    poll_until_finished(&mut app, &mut out);
    assert!(marker.exists());
    assert!(app.llm_preview.is_some(), "status: {:?}", app.message);
    assert_eq!(
        fs::read_to_string(repo.0.join("note.txt")).unwrap(),
        "one\ntwo\n"
    );

    app.handle_key_with(&mut out, key(KeyCode::Enter)).unwrap();
    poll_until_finished(&mut app, &mut out);
    assert_eq!(app.buffer.to_string(), "one\nTWO\n");
    assert_eq!(
        fs::read_to_string(repo.0.join("note.txt")).unwrap(),
        "one\ntwo\n"
    );
    let _ = fs::remove_dir_all(command_root);
}

#[test]
fn patch_for_a_different_repo_file_is_refused_before_preview() {
    let repo = TempRepo::new();
    let patch = "--- a/other.txt\n+++ b/other.txt\n@@ -1 +1 @@\n-stable\n+changed\n";
    let (settings, server) = response_server(patch);
    let mut app = project_app(&repo);
    let original = app.buffer.to_string();
    let mut out = Vec::new();
    begin_with_settings(&mut app, &mut out, "uppercase second line", settings).unwrap();
    poll_until_pending(&mut app, &mut out);

    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
    poll_until_finished(&mut app, &mut out);
    server.join().unwrap();

    assert!(app.llm_preview.is_none());
    assert_eq!(app.buffer.to_string(), original);
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("other than active path note.txt"));
}

fn project_app(repo: &TempRepo) -> super::super::App {
    project_app_at(&repo.0.join("note.txt"))
}

fn project_app_at(path: &Path) -> super::super::App {
    let mut app = super::super::App::new(path.to_str()).unwrap();
    let mut out = Vec::new();
    super::super::project_mode::switch_to_project(&mut app, &mut out).unwrap();
    app
}

fn poll_until_pending(app: &mut super::super::App, out: &mut Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while matches!(
        app.repo_llm_state.as_ref(),
        Some(RepoLlmState::Preparing(_))
    ) {
        poll(app, out).unwrap();
        assert!(Instant::now() < deadline, "repo preparation timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn poll_until_finished(app: &mut super::super::App, out: &mut Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.repo_llm_state.is_some() {
        poll(app, out).unwrap();
        assert!(Instant::now() < deadline, "repo request timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn poll_until_running(app: &mut super::super::App, out: &mut Vec<u8>) {
    poll_until_send_checked(app, out);
    assert!(matches!(
        app.repo_llm_state.as_ref(),
        Some(RepoLlmState::Running(_))
    ));
}

fn poll_until_send_checked(app: &mut super::super::App, out: &mut Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while matches!(
        app.repo_llm_state.as_ref(),
        Some(RepoLlmState::CheckingSend(_))
    ) {
        poll(app, out).unwrap();
        assert!(Instant::now() < deadline, "repo drift check timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn patch_server() -> (BackendPreset, std::thread::JoinHandle<()>) {
    response_server("--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n")
}

fn response_server(patch: &str) -> (BackendPreset, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let patch = patch.to_string();
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

fn settings(base_url: String) -> BackendPreset {
    crate::config::llm::parse(&format!(
        "[llm]\nbase_url={base_url:?}\nmodel='test-model'\ntimeout_secs=2\n"
    ))
    .unwrap()
    .default_preset()
    .clone()
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

struct TempRepo(PathBuf);

impl TempRepo {
    fn new() -> Self {
        let suffix = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "catomic-app-repo-llm-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        git(&path, &["init", "-q", "-b", "main"]);
        fs::write(path.join("note.txt"), "one\ntwo\n").unwrap();
        fs::write(path.join("other.txt"), "stable\n").unwrap();
        git(&path, &["add", "."]);
        git(
            &path,
            &[
                "-c",
                "user.name=Catomic Test",
                "-c",
                "user.email=catomic@example.invalid",
                "commit",
                "-q",
                "-m",
                "initial",
            ],
        );
        Self(path)
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(root: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .unwrap()
        .success());
}
