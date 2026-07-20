//! Purpose: prove picker filtering, narrow rendering, cancellation, selection, and no invocation.
//! Owns: App-level preset fixtures and loopback listeners that must remain untouched on open.
//! Must not: read user config, read credential values, launch CLIs, or contact public network.
//! Invariants: selection is session-wide and neither picker open nor Enter rewrites config.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;

fn catalog(endpoint: &str) -> LlmCatalog {
    crate::config::llm::parse(&format!(
        r#"
[llm]
default = "local"
[[llm.backends]]
name = "local"
type = "openai-compatible"
base_url = "{endpoint}"
model = "base"
models = ["small", "猫-large"]
discovery = true
headers = {{ "X-Test-Metadata" = "PICKER_MUST_NOT_RENDER_THIS_VALUE" }}
[[llm.backends]]
name = "hosted"
type = "openai-compatible"
base_url = "https://models.example/v1"
model = "remote-model"
api_key_env = "CATOMIC_ISSUE_67_PICKER_MISSING_KEY"
[[llm.backends]]
name = "missing-cli"
type = "command"
program = "catomic-missing-picker-test"
args = ["space arg", "猫"]
model = "cli-model"
output = "claude-json-v1"
[[llm.backends]]
name = "available-cli"
type = "command"
program = "sh"
model = "available-cli-model"
output = "codex-jsonl-v1"
"#
    ))
    .unwrap()
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn opening_filtering_and_selecting_never_contacts_or_starts_backend() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    show_with_catalog(&mut app, &mut out, catalog(&endpoint)).unwrap();

    assert!(
        listener.accept().is_err(),
        "picker open must not contact provider"
    );
    let text = display_buffer(&app).unwrap().to_string();
    assert!(text.contains("[A-D] local | base"));
    assert!(text.contains("missing executable"));
    assert!(text.contains("missing credential CATOMIC_ISSUE_67_PICKER_MISSING_KEY"));
    assert!(!text.contains("PICKER_MUST_NOT_RENDER_THIS_VALUE"));

    handle_key(&mut app, &mut out, key(KeyCode::Char('猫'))).unwrap();
    assert_eq!(app.model_picker.view.as_ref().unwrap().visible.len(), 1);
    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();

    assert!(!is_viewing(&app));
    assert_eq!(app.model_session.selected().unwrap().model, "猫-large");
    assert!(
        listener.accept().is_err(),
        "selection must not contact provider"
    );
}

#[test]
fn session_selection_survives_buffer_switch_and_picker_cancel() {
    let mut app = super::super::App::new_with_paths_and_big_file_config(
        &[String::new(), String::new()],
        crate::config::big_files::BigFileConfig::default(),
    )
    .unwrap();
    let mut out = Vec::new();
    let catalog = catalog("http://127.0.0.1:9/v1");
    show_with_catalog(&mut app, &mut out, catalog.clone()).unwrap();
    handle_key(&mut app, &mut out, key(KeyCode::Down)).unwrap();
    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
    assert_eq!(app.model_session.selected().unwrap().model, "small");

    app.switch_buffer(super::super::buffers::BufferDirection::Next);
    assert_eq!(app.model_session.selected().unwrap().model, "small");
    show_with_catalog(&mut app, &mut out, catalog).unwrap();
    handle_key(&mut app, &mut out, key(KeyCode::Esc)).unwrap();
    assert_eq!(app.model_session.selected().unwrap().model, "small");
}

#[test]
fn selecting_a_model_disables_prior_autocomplete_authorization() {
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    let first = catalog("http://127.0.0.1:9/v1");
    super::super::autocomplete::begin_with_catalog(&mut app, &mut out, first.clone()).unwrap();
    super::super::autocomplete::handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
    assert!(app.autocomplete.enabled);
    assert!(app.autocomplete.confirmed.is_some());

    show_with_catalog(&mut app, &mut out, first).unwrap();
    handle_key(&mut app, &mut out, key(KeyCode::Down)).unwrap();
    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();

    assert!(!app.autocomplete.enabled);
    assert!(app.autocomplete.confirmed.is_none());
}

#[test]
fn picker_owns_and_restores_the_complete_viewport() {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("a\nb\nc\nsource"));
    app.buffer
        .set_cursor(crate::buffer::Cursor { row: 3, col: 3 });
    app.view.soft_wrap = true;
    app.screen.width = 4;
    app.screen.height = 2;
    app.screen.scroll_top = 3;
    app.screen.scroll_left = 0;
    app.screen.wrap_col = 2;
    let mut out = Vec::new();

    show_with_catalog(&mut app, &mut out, catalog("http://127.0.0.1:9/v1")).unwrap();
    assert_eq!(
        (
            app.screen.scroll_top,
            app.screen.scroll_left,
            app.screen.wrap_col
        ),
        (0, 0, 0)
    );

    handle_key(&mut app, &mut out, key(KeyCode::Esc)).unwrap();
    assert_eq!(
        (
            app.screen.scroll_top,
            app.screen.scroll_left,
            app.screen.wrap_col
        ),
        (3, 0, 2)
    );
}

#[test]
fn picker_keeps_an_orphaned_session_override_visible_and_active() {
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    show_with_catalog(&mut app, &mut out, catalog("http://127.0.0.1:9/v1")).unwrap();
    handle_key(&mut app, &mut out, key(KeyCode::Down)).unwrap();
    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();

    let changed = crate::config::llm::parse(
        "[[llm.backends]]\nname='different'\ntype='openai-compatible'\nbase_url='http://127.0.0.1:8/v1'\nmodel='new-default'\n",
    )
    .unwrap();
    show_with_catalog(&mut app, &mut out, changed).unwrap();
    let text = display_buffer(&app).unwrap().to_string();
    assert!(text.contains("[AS-] local | small"));
    assert!(text.contains("session override"));
}

#[test]
fn remapped_select_model_action_opens_the_picker() {
    let mut app = super::super::App::new(None).unwrap();
    app.keybindings =
        crate::config::keybindings::parse("[keybindings]\n'alt+m' = 'select-model'\n").unwrap();
    let mut out = Vec::new();

    app.handle_key_with(
        &mut out,
        KeyEvent::new(KeyCode::Char('m'), KeyModifiers::ALT),
    )
    .unwrap();

    assert!(is_viewing(&app));
}

#[test]
fn narrow_terminal_clips_safely_and_discovery_requires_second_confirmation() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let mut app = super::super::App::new(None).unwrap();
    app.screen.width = 12;
    app.screen.height = 3;
    let mut out = Vec::new();
    show_with_catalog(&mut app, &mut out, catalog(&endpoint)).unwrap();
    assert!(String::from_utf8_lossy(&out).contains("[A-D] local"));

    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert!(app
        .model_picker
        .view
        .as_ref()
        .unwrap()
        .pending_discovery
        .is_some());
    assert!(listener.accept().is_err());
    handle_key(&mut app, &mut out, key(KeyCode::Esc)).unwrap();
    assert!(is_viewing(&app));
    assert!(app
        .model_picker
        .view
        .as_ref()
        .unwrap()
        .pending_discovery
        .is_none());
    assert!(app.message.is_none());
    assert!(listener.accept().is_err());
}

#[test]
fn picker_displays_complete_large_discovered_catalog() {
    let endpoint = "http://127.0.0.1:9/v1";
    let catalog = catalog(endpoint);
    let cache_key = cache_key(catalog.default_preset());
    let models = (0..300)
        .map(|index| format!("discovered-{index:03}"))
        .collect();
    let mut app = super::super::App::new(None).unwrap();
    app.model_picker.cache.insert(
        cache_key,
        CachedModels {
            models,
            expires: Instant::now() + CACHE_TTL,
        },
    );
    let mut out = Vec::new();

    show_with_catalog(&mut app, &mut out, catalog).unwrap();

    let text = display_buffer(&app).unwrap().to_string();
    assert!(text.contains("discovered-000"));
    assert!(text.contains("discovered-150"));
    assert!(text.contains("discovered-299"));
}

#[test]
fn confirmed_discovery_adds_validated_models_and_reuses_session_cache() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let count = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..count]);
        assert!(request.starts_with("GET /v1/models"));
        assert!(request.contains("x-test-metadata: PICKER_MUST_NOT_RENDER_THIS_VALUE"));
        let body = r#"{"data":[{"id":"discovered-one"},{"id":"猫-remote"}]}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    let catalog = catalog(&endpoint);
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    show_with_catalog(&mut app, &mut out, catalog.clone()).unwrap();
    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
    )
    .unwrap();
    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.model_picker.discovery.is_some() {
        poll(&mut app, &mut out).unwrap();
        assert!(Instant::now() < deadline, "discovery timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
    server.join().unwrap();
    assert!(display_buffer(&app)
        .unwrap()
        .to_string()
        .contains("discovered-one"));

    close(&mut app);
    show_with_catalog(&mut app, &mut out, catalog).unwrap();
    let text = display_buffer(&app).unwrap().to_string();
    assert!(text.contains("discovered-one"));
    assert!(text.contains("ready (cached discovery)"));
}

#[test]
fn picker_distinguishes_an_incompatible_structured_cli_after_failure() {
    let catalog = catalog("http://127.0.0.1:9/v1");
    let mut app = super::super::App::new(None).unwrap();
    app.model_session.record_failure(
        "available-cli",
        crate::llm::backend::BackendErrorKind::Incompatible,
    );
    let mut out = Vec::new();
    show_with_catalog(&mut app, &mut out, catalog).unwrap();

    assert!(display_buffer(&app)
        .unwrap()
        .to_string()
        .contains("incompatible CLI output/version"));
}
