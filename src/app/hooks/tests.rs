//! Purpose: prove lifecycle hook order, save integration, failure abort, and LLM deferral.
//! Owns: deterministic App-level hooks using local shell commands and no live endpoint.
//! Must not: send network requests, modify repository files, or bypass command preview.
//! Invariants: hooks are sequential; failure stops; before-LLM success only reaches confirmation.
//! Phase: 7 hooks acceptance.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;

fn configure(app: &mut super::super::App, text: &str) {
    app.command_config = crate::config::commands::parse(text).unwrap();
}

fn wait_for_preview(app: &mut super::super::App, out: &mut Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !super::super::external_command::is_viewing(app) {
        super::super::external_command::poll(app, out).unwrap();
        assert!(Instant::now() < deadline, "hook command timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn close_result(app: &mut super::super::App, out: &mut Vec<u8>) {
    super::super::external_command::handle_key(
        app,
        out,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .unwrap();
}

#[test]
fn open_hooks_run_sequentially_in_configuration_order() {
    let mut app = super::super::App::new(None).unwrap();
    configure(
        &mut app,
        "[commands.first]\ncommand = \"printf first\"\n\
         [commands.second]\ncommand = \"printf second\"\n\
         [hooks]\non_open = [\"first\", \"second\"]\n",
    );
    let mut out = Vec::new();

    trigger_open(&mut app);
    pump(&mut app, &mut out).unwrap();
    assert_eq!(app.hooks.active.as_deref(), Some("first"));
    wait_for_preview(&mut app, &mut out);
    close_result(&mut app, &mut out);

    pump(&mut app, &mut out).unwrap();
    assert_eq!(app.hooks.active.as_deref(), Some("second"));
    wait_for_preview(&mut app, &mut out);
    close_result(&mut app, &mut out);
    pump(&mut app, &mut out).unwrap();

    assert!(!is_pending(&app));
}

#[test]
fn successful_save_queues_on_save_hook() {
    let path = std::env::temp_dir().join(format!("catomic_hook_save_{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut app = super::super::App::new(Some(path.to_str().unwrap())).unwrap();
    configure(
        &mut app,
        "[commands.saved]\ncommand = \"printf saved\"\n[hooks]\non_save = [\"saved\"]\n",
    );
    app.buffer.insert_char('x');

    super::super::save::handle_save(&mut app, &mut Vec::new()).unwrap();

    assert!(is_pending(&app));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x");
    let _ = std::fs::remove_file(path);
}

#[test]
fn failed_atomic_save_does_not_queue_on_save_hook() {
    let target =
        std::env::temp_dir().join(format!("catomic_hook_failed_save_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&target);
    std::fs::create_dir(&target).unwrap();
    let mut app = super::super::App::new(None).unwrap();
    app.file.path = Some(target.clone());
    app.file.disk_snapshot = crate::file::io::capture_file_snapshot(&target).ok();
    configure(
        &mut app,
        "[commands.saved]\ncommand = \"printf saved\"\n[hooks]\non_save = [\"saved\"]\n",
    );
    app.buffer.insert_char('x');

    super::super::save::handle_save(&mut app, &mut Vec::new()).unwrap();

    assert!(!is_pending(&app));
    assert!(app.message.as_deref().unwrap().contains("Save error"));
    std::fs::remove_dir(&target).unwrap();
}

#[test]
fn failed_before_llm_hook_aborts_without_request_or_network_task() {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer.insert_char('x');
    configure(
        &mut app,
        "[commands.guard]\ncommand = \"printf denied; exit 7\"\n\
         [hooks]\nbefore_llm = [\"guard\"]\n",
    );
    let mut out = Vec::new();

    before_current_llm(
        &mut app,
        &mut out,
        super::super::llm_request::CurrentLlmCommand::BigMeow,
        "explain",
    )
    .unwrap();
    pump(&mut app, &mut out).unwrap();
    wait_for_preview(&mut app, &mut out);
    close_result(&mut app, &mut out);
    pump(&mut app, &mut out).unwrap();

    assert!(!is_pending(&app));
    assert!(app.pending_llm_request.is_none());
    assert!(app.llm_task.is_none());
}

#[test]
fn successful_before_llm_hook_resumes_only_to_local_confirmation() {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer.insert_char('x');
    configure(
        &mut app,
        "[commands.guard]\ncommand = \"printf ok\"\n\
         [hooks]\nbefore_llm = [\"guard\"]\n",
    );
    let mut out = Vec::new();

    before_current_llm(
        &mut app,
        &mut out,
        super::super::llm_request::CurrentLlmCommand::BigMeow,
        "explain",
    )
    .unwrap();
    pump(&mut app, &mut out).unwrap();
    wait_for_preview(&mut app, &mut out);
    close_result(&mut app, &mut out);
    pump(&mut app, &mut out).unwrap();

    assert!(!is_pending(&app));
    assert!(app.pending_llm_request.is_some());
    assert!(app.llm_task.is_none());
}
