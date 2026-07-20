//! Purpose: verify request debounce, stale-response rejection, cancellation, and backoff.
//! Owns: identity-drift fixtures and one private loopback cancellation server.
//! Must not: contact a live/public endpoint, read repositories, save files, or use credentials.
//! Invariants: late/cancelled work cannot display or mutate a suggestion.

use std::io::Read;
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::buffer::Cursor;
use crate::llm::task::{LlmTask, LlmTaskResult};

#[test]
fn stale_response_is_discarded_for_cursor_drift() {
    let mut app = enabled_app("prefix", Cursor { row: 0, col: 6 });
    let policy = app.autocomplete.confirmed.as_ref().unwrap().clone();
    let identity = request::current_identity(&app, &policy);
    app.buffer.set_cursor(Cursor { row: 0, col: 0 });
    let mut out = Vec::new();

    request::finish_for_test(
        &mut app,
        &mut out,
        identity,
        LlmTaskResult::Finished("late text".to_string()),
    )
    .unwrap();

    assert!(app.autocomplete.suggestion.is_none());
    assert_eq!(app.buffer.to_string(), "prefix");
}

#[test]
fn malformed_result_enters_bounded_retry_backoff() {
    let mut app = enabled_app("prefix", Cursor { row: 0, col: 6 });
    let policy = app.autocomplete.confirmed.as_ref().unwrap().clone();
    let identity = request::current_identity(&app, &policy);
    let mut out = Vec::new();

    request::finish_for_test(
        &mut app,
        &mut out,
        identity,
        LlmTaskResult::Finished("\u{1b}[2J".to_string()),
    )
    .unwrap();

    assert!(app.autocomplete.suggestion.is_none());
    assert!(app.autocomplete.backoff_until.is_some());
    assert_eq!(status_label(&app), "ac error");
}

#[test]
fn identity_pins_revision_mode_generation_endpoint_and_model() {
    let app = enabled_app("prefix", Cursor { row: 0, col: 6 });
    let policy = app.autocomplete.confirmed.as_ref().unwrap().clone();
    let identity = request::current_identity(&app, &policy);
    assert!(request::identity_is_current(&app, &identity));

    let mut revision_drift = enabled_app("prefix", Cursor { row: 0, col: 6 });
    revision_drift.buffer.insert_char('!');
    assert!(!request::identity_is_current(&revision_drift, &identity));

    let mut mode_drift = enabled_app("prefix", Cursor { row: 0, col: 6 });
    mode_drift.mode = crate::mode::Mode::Project;
    assert!(!request::identity_is_current(&mode_drift, &identity));

    let mut generation_drift = enabled_app("prefix", Cursor { row: 0, col: 6 });
    generation_drift.autocomplete.generation += 1;
    assert!(!request::identity_is_current(&generation_drift, &identity));

    let mut destination_drift = enabled_app("prefix", Cursor { row: 0, col: 6 });
    mutate_confirmed_http_destination(&mut destination_drift, "http://localhost:9999/v1");
    assert!(!request::identity_is_current(&destination_drift, &identity));

    let mut model_drift = enabled_app("prefix", Cursor { row: 0, col: 6 });
    mutate_confirmed_model(&mut model_drift, "other-model");
    assert!(!request::identity_is_current(&model_drift, &identity));
}

#[test]
fn debounce_and_backoff_gate_request_start_without_blocking() {
    let mut app = enabled_app("prefix", Cursor { row: 0, col: 6 });
    let now = Instant::now();
    app.autocomplete.last_edit = Some(now);
    assert!(!request::should_start_for_test(&app, now));

    app.autocomplete.last_edit = Some(now - Duration::from_secs(1));
    assert!(request::should_start_for_test(&app, now));

    app.autocomplete.backoff_until = Some(now + Duration::from_secs(1));
    assert!(!request::should_start_for_test(&app, now));
}

#[test]
fn ordinary_navigation_invalidates_ready_text_immediately() {
    let mut app = enabled_app("prefix", Cursor { row: 0, col: 6 });
    ready(&mut app, " continuation");
    let generation = app.autocomplete.generation;
    let mut out = Vec::new();

    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
        .unwrap();

    assert!(app.autocomplete.suggestion.is_none());
    assert!(app.autocomplete.generation > generation);
}

#[test]
fn selection_change_invalidates_ready_text_before_highlighting() {
    let mut app = enabled_app("prefix", Cursor { row: 0, col: 6 });
    ready(&mut app, " continuation");
    let mut out = Vec::new();

    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT))
        .unwrap();

    assert!(app.autocomplete.suggestion.is_none());
    assert!(app.selection.active().is_some());
}

#[test]
fn invalidation_drops_and_cancels_an_in_flight_worker() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted, accepted_rx) = mpsc::sync_channel(1);
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        accepted.send(()).unwrap();
        let mut bytes = [0u8; 512];
        loop {
            match stream.read(&mut bytes) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    });
    let mut app = enabled_app("prefix", Cursor { row: 0, col: 6 });
    let policy = app.autocomplete.confirmed.as_ref().unwrap().clone();
    let identity = request::current_identity(&app, &policy);
    let catalog =
        local_catalog_at_with_timeout(&format!("http://{address}/v1"), Duration::from_secs(5));
    let task = LlmTask::start_bounded(
        selected_backend(&catalog),
        "system".to_string(),
        "user".to_string(),
        8,
    )
    .unwrap();
    app.autocomplete.running = Some(RunningRequest { task, identity });
    accepted_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    invalidate(&mut app);

    assert!(app.autocomplete.running.is_none());
    server.join().unwrap();
}
