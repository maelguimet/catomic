//! Purpose: verify autocomplete opt-in, scoped confirmation, acceptance, and key behavior.
//! Owns: deterministic App fixtures, session policy, undo, and line-ending acceptance tests.
//! Must not: contact a live/public endpoint, read repositories, or use real credentials.
//! Invariants: ghost text is non-buffer state; acceptance is one undoable transaction.
//! Phase: post-v0.1 opt-in inline autocomplete.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::buffer::{Cursor, PieceTable};
use crate::config::llm::{BackendAdapter, LlmCatalog};

mod request_cases;

fn local_catalog() -> LlmCatalog {
    let mut catalog = LlmCatalog::default();
    let preset = &mut catalog.presets[0];
    preset.model = "test-writer".to_string();
    let BackendAdapter::OpenAiCompatible(http) = &mut preset.adapter else {
        panic!("default preset must use HTTP");
    };
    http.base_url = "http://127.0.0.1:8080/v1".to_string();
    http.api_key_env = Some("CATOMIC_AUTOCOMPLETE_TEST_KEY".to_string());
    http.timeout = Duration::from_secs(2);
    catalog
}

fn confirmation_text(app: &super::super::App) -> String {
    app.autocomplete
        .pending
        .as_ref()
        .expect("pending confirmation")
        .buffer
        .to_string()
}

fn set_http_destination(catalog: &mut LlmCatalog, destination: &str) {
    let BackendAdapter::OpenAiCompatible(http) = &mut catalog.presets[0].adapter else {
        panic!("test preset must use HTTP");
    };
    http.base_url = destination.to_string();
}

fn set_http_key(catalog: &mut LlmCatalog, key_env: Option<&str>) {
    let BackendAdapter::OpenAiCompatible(http) = &mut catalog.presets[0].adapter else {
        panic!("test preset must use HTTP");
    };
    http.api_key_env = key_env.map(str::to_string);
    http.credential_required = false;
}

fn set_http_timeout(catalog: &mut LlmCatalog, timeout: Duration) {
    let BackendAdapter::OpenAiCompatible(http) = &mut catalog.presets[0].adapter else {
        panic!("test preset must use HTTP");
    };
    http.timeout = timeout;
}

fn no_key_local_catalog() -> LlmCatalog {
    let mut catalog = local_catalog();
    set_http_key(&mut catalog, None);
    catalog
}

fn local_catalog_at(destination: &str) -> LlmCatalog {
    let mut catalog = no_key_local_catalog();
    set_http_destination(&mut catalog, destination);
    catalog
}

fn local_catalog_at_with_timeout(destination: &str, timeout: Duration) -> LlmCatalog {
    let mut catalog = local_catalog_at(destination);
    set_http_timeout(&mut catalog, timeout);
    catalog
}

fn selected_policy(catalog: &LlmCatalog) -> BackendPreset {
    catalog.default_preset().clone()
}

fn selected_backend(catalog: &LlmCatalog) -> crate::llm::backend::ConfirmedBackend {
    crate::llm::backend::ConfirmedBackend::resolve(&selected_policy(catalog)).unwrap()
}

fn mutate_confirmed_http_destination(app: &mut super::super::App, destination: &str) {
    let policy = app.autocomplete.confirmed.as_mut().unwrap();
    policy.destination = destination.to_string();
}

fn mutate_confirmed_model(app: &mut super::super::App, model: &str) {
    app.autocomplete.confirmed.as_mut().unwrap().preset.model = model.to_string();
}

fn enabled_app(text: &str, cursor: Cursor) -> super::super::App {
    let mut app = super::super::App::new(None).unwrap();
    app.autocomplete.config.minimum_prefix_length = 1;
    app.buffer = Box::new(PieceTable::from_text(text));
    app.buffer.set_cursor(cursor);
    let mut out = Vec::new();
    confirm_local(&mut app, &mut out);
    assert!(app.autocomplete.running.is_none());
    app
}

fn ready(app: &mut super::super::App, text: &str) {
    let policy = app.autocomplete.confirmed.as_ref().unwrap().clone();
    let identity = request::current_identity(app, &policy);
    app.autocomplete.suggestion = Some(Suggestion {
        text: text.to_string(),
        identity,
    });
}

fn confirm_local(app: &mut super::super::App, out: &mut Vec<u8>) {
    begin_with_catalog(app, out, local_catalog()).unwrap();
    handle_key(app, out, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).unwrap();
    app.message = None;
}

#[test]
fn disabled_default_constructs_no_pending_or_running_request() {
    let app = super::super::App::new(None).unwrap();
    assert!(!app.autocomplete.enabled);
    assert!(app.autocomplete.pending.is_none());
    assert!(app.autocomplete.running.is_none());
    assert_eq!(status_label(&app), "autocomplete disabled");
}

#[test]
fn first_enable_discloses_destination_scope_and_requires_enter() {
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();

    begin_with_catalog(&mut app, &mut out, local_catalog()).unwrap();

    assert!(!app.autocomplete.enabled);
    assert!(app.autocomplete.pending.is_some());
    assert!(app.autocomplete.running.is_none());
    let document = confirmation_text(&app);
    assert!(document.contains("Preset: local"));
    assert!(document.contains("Model: test-writer"));
    assert!(document.contains("http://127.0.0.1:8080/v1"));
    assert!(document.contains("2048 Unicode scalars before and 512 after"));
    assert!(document.contains("no repository or filesystem context"));
    assert!(document.contains("No credential is read"));

    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .unwrap();
    assert!(app.autocomplete.enabled);
    assert!(app.autocomplete.confirmed.is_some());
    assert!(app.autocomplete.running.is_none());
}

#[test]
fn remote_endpoint_requires_separate_configuration_before_confirmation() {
    let mut app = super::super::App::new(None).unwrap();
    let mut catalog = local_catalog();
    set_http_destination(&mut catalog, "https://models.example/v1");
    let mut out = Vec::new();

    begin_with_catalog(&mut app, &mut out, catalog).unwrap();

    assert!(!app.autocomplete.enabled);
    assert!(app.autocomplete.pending.is_none());
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("allow_remote = true"));
}

#[test]
fn separately_allowed_remote_endpoint_still_requires_explicit_confirmation() {
    let mut app = super::super::App::new(None).unwrap();
    app.autocomplete.config.allow_remote = true;
    let mut catalog = local_catalog();
    set_http_destination(&mut catalog, "https://models.example/v1");
    let mut out = Vec::new();

    begin_with_catalog(&mut app, &mut out, catalog).unwrap();

    assert!(!app.autocomplete.enabled);
    assert!(app.autocomplete.pending.is_some());
    assert!(app.autocomplete.running.is_none());
    assert!(confirmation_text(&app).contains("(REMOTE HTTP)"));
}

#[test]
fn tab_accepts_ready_text_as_one_undoable_edit() {
    let mut app = enabled_app("Hello ", Cursor { row: 0, col: 6 });
    ready(&mut app, "wide 猫🙂\nnext");
    let before_revision = app.buffer.edit_history_position();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "Hello wide 猫🙂\nnext");
    assert_ne!(app.buffer.edit_history_position(), before_revision);
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "Hello ");
}

#[test]
fn escape_dismisses_without_buffer_or_history_change() {
    let mut app = enabled_app("Hello", Cursor { row: 0, col: 5 });
    ready(&mut app, " world");
    let revision = app.buffer.edit_history_position();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "Hello");
    assert_eq!(app.buffer.edit_history_position(), revision);
    assert!(app.autocomplete.suggestion.is_none());
}

#[test]
fn tab_keeps_normal_indentation_behavior_without_a_visible_suggestion() {
    let mut app = enabled_app("x", Cursor { row: 0, col: 1 });
    let mut out = Vec::new();

    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "x   ");
}

#[test]
fn tab_does_not_accept_or_consume_a_hidden_stale_suggestion() {
    let mut app = enabled_app("prefix", Cursor { row: 0, col: 6 });
    ready(&mut app, " continuation");
    app.buffer.set_cursor(Cursor { row: 0, col: 0 });
    let mut out = Vec::new();

    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "    prefix");
    assert!(app.autocomplete.suggestion.is_none());
}

#[test]
fn accepted_newlines_save_using_the_existing_crlf_format() {
    let path = std::env::temp_dir().join(format!(
        "catomic_autocomplete_crlf_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, b"Hello\r\n").unwrap();
    let mut app = super::super::App::new(path.to_str()).unwrap();
    app.buffer.set_cursor(Cursor { row: 0, col: 5 });
    let mut out = Vec::new();
    confirm_local(&mut app, &mut out);
    ready(&mut app, "\nnext");

    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(
        &mut out,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), b"Hello\r\nnext\r\n");
    drop(app);
    let _ = std::fs::remove_file(path);
}
