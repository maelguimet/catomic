//! Purpose: verify the integrated F3 inline-clanker state machine and safety gates.
//! Owns: App fixtures, fake command adapters, queue/apply/highlight regression tests.
//! Must not: contact endpoints, save files, or duplicate pure discovery coverage.
//! Invariants: adapters stay process-local and every edit still requires two confirmations.
//! Phase: issue #65 one-key inline clanker workflow.

use std::sync::atomic::Ordering;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};

use super::*;
use crate::buffer::{Buffer, Cursor};
use crate::config::llm::{InlineBlockMode, LlmCatalog as LlmSettings};

mod support;
use support::*;

#[test]
fn f3_discovers_inline_scope_without_connecting_before_confirmation() {
    let (mut settings, _, accepted, server) = tracked_response_server(Vec::new());
    settings.inline.warn_lines = 500;
    let mut app = app_with(">> Rewrite\n<catblock>\ncat\n</catblock>\n");
    app.screen.scroll_top = 2;
    app.screen.scroll_left = 1;
    app.screen.wrap_col = 3;
    let mut out = Vec::new();

    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();

    assert!(matches!(app.inline_clanker.phase, Some(Phase::Confirm(_))));
    let confirmation = display_buffer(&app).unwrap().to_string();
    assert!(confirmation.contains("Instruction source: line 1"));
    assert!(confirmation.contains("Scope: 1 context block"));
    assert!(confirmation.contains("Request 1:"));
    assert!(confirmation.contains("No client, process, or network request starts until Enter."));
    assert_eq!(app.screen.scroll_top, 0);
    assert_eq!(app.screen.scroll_left, 0);
    assert_eq!(app.screen.wrap_col, 0);
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        0,
        "confirmation must not invoke the backend"
    );

    app.handle_key_with(&mut out, key(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.screen.scroll_top, 2);
    assert_eq!(app.screen.scroll_left, 1);
    assert_eq!(app.screen.wrap_col, 3);
    app.handle_key_with(&mut out, key(KeyCode::F(3), KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(app.inline_clanker.phase, Some(Phase::Confirm(_))));
    server.join().unwrap();
}

#[test]
fn proposal_preview_restores_the_source_wrap_viewport_when_closed() {
    let (settings, _, server) =
        response_server(vec![r#"{"catomic_replacement":"CAT\n"}"#.to_string()]);
    let mut app = app_with(">> Rewrite\n<catblock>\ncat\n</catblock>\n");
    app.view.soft_wrap = true;
    app.screen.scroll_top = 2;
    app.screen.scroll_left = 1;
    app.screen.wrap_col = 3;
    let mut out = Vec::new();

    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    poll_until_not_running(&mut app, &mut out);

    assert!(matches!(app.inline_clanker.phase, Some(Phase::Preview(_))));
    assert_eq!(app.screen.scroll_top, 0);
    assert_eq!(app.screen.scroll_left, 0);
    assert_eq!(app.screen.wrap_col, 0);
    assert!(cancel_all(&mut app));
    assert_eq!(app.screen.scroll_top, 2);
    assert_eq!(app.screen.scroll_left, 1);
    assert_eq!(app.screen.wrap_col, 3);
    server.join().unwrap();
}

#[test]
fn combined_blocks_preview_apply_cleanup_undo_redo_and_highlight_atomically() {
    let responses = vec![
        r#"{"catomic_replacements":[{"block":1,"replacement":"ONE\n"},{"block":2,"replacement":"TWO\n"}]}"#
            .to_string(),
    ];
    let (settings, requests, server) = response_server(responses);
    let original =
        ">> Rename\n<catblock>\none\n</catblock>\nPRIVATE\n<catblock>\ntwo\n</catblock>\n";
    let mut app = app_with(original);
    let mut out = Vec::new();
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();

    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    poll_until_not_running(&mut app, &mut out);
    assert!(matches!(app.inline_clanker.phase, Some(Phase::Preview(_))));
    let preview = match app.inline_clanker.phase.as_ref().unwrap() {
        Phase::Preview(preview) => preview.buffer.to_string(),
        _ => unreachable!(),
    };
    assert!(preview.contains("context block 1 lines 3-3"));
    assert!(preview.contains("confirmed instruction cleanup line 1"));
    assert!(preview.contains("-one"));
    assert!(preview.contains("+ONE"));
    assert_eq!(app.buffer.to_string(), original);

    out.clear();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    let applied = "<catblock>\nONE\n</catblock>\nPRIVATE\n<catblock>\nTWO\n</catblock>\n";
    assert_eq!(app.buffer.to_string(), applied);
    assert!(app.file.dirty);
    assert!(source_changes(&app).is_some());
    let rendered = String::from_utf8_lossy(&out);
    assert!(
        rendered.contains("\x1b[31;4m"),
        "model text should use semantic red; got {rendered:?}"
    );
    assert!(
        rendered.contains("\x1b[31;1;4m┃"),
        "touched lines need a gutter mark"
    );

    app.handle_key_with(&mut out, key(KeyCode::Char('z'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.buffer.to_string(), original);
    assert!(source_changes(&app).is_none());
    app.handle_key_with(&mut out, key(KeyCode::Char('y'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.buffer.to_string(), applied);
    assert!(source_changes(&app).is_some());

    server.join().unwrap();
    let request = &requests.lock().unwrap()[0];
    assert!(request.contains("Context block 1 of 2"));
    assert!(!request.contains("PRIVATE"));
    assert!(!request.contains("<catblock>"));
}

#[test]
fn selected_context_sends_and_edits_only_the_exact_selection() {
    let (settings, requests, server) =
        response_server(vec![r#"{"catomic_replacement":"PUBLIC"}"#.to_string()]);
    let mut app = app_with(">> Uppercase\nPRIVATE\npublic\nafter\n");
    app.buffer.set_cursor(Cursor { row: 2, col: 0 });
    let mut out = Vec::new();
    for _ in 0..6 {
        app.handle_key_with(&mut out, key(KeyCode::Right, KeyModifiers::SHIFT))
            .unwrap();
    }
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    poll_until_not_running(&mut app, &mut out);
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "PRIVATE\nPUBLIC\nafter\n");
    server.join().unwrap();
    let request = &requests.lock().unwrap()[0];
    assert!(request.contains("Context:\npublic"));
    assert!(!request.contains("PRIVATE"));
    assert!(!request.contains("public\\nafter"));
}

#[test]
fn queued_blocks_start_only_after_each_apply_and_undo_separately() {
    let (mut settings, requests, accepted, server) = tracked_response_server(vec![
        r#"{"catomic_replacement":"ONE\n"}"#.to_string(),
        r#"{"catomic_replacement":"TWO\n"}"#.to_string(),
    ]);
    settings.inline.block_mode = InlineBlockMode::Queued;
    let original = ">> Rename\n<catblock>\none\n</catblock>\n<catblock>\ntwo\n</catblock>\n";
    let mut app = app_with(original);
    let mut out = Vec::new();
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    poll_until_not_running(&mut app, &mut out);
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
    assert_eq!(requests.lock().unwrap().len(), 1);

    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    wait_until(|| accepted.load(Ordering::SeqCst) == 2);
    assert!(app.message.as_deref().unwrap().contains("block 2/2"));
    poll_until_not_running(&mut app, &mut out);
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(
        app.buffer.to_string(),
        "<catblock>\nONE\n</catblock>\n<catblock>\nTWO\n</catblock>\n"
    );

    app.handle_key_with(&mut out, key(KeyCode::Char('z'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(
        app.buffer.to_string(),
        ">> Rename\n<catblock>\nONE\n</catblock>\n<catblock>\ntwo\n</catblock>\n"
    );
    assert!(source_changes(&app).is_some(), "first block remains marked");
    app.handle_key_with(&mut out, key(KeyCode::Char('z'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.buffer.to_string(), original);
    assert!(source_changes(&app).is_none());
    server.join().unwrap();
}

#[test]
fn queued_cancellation_keeps_instruction_and_clears_remaining_work() {
    let (mut settings, _requests, accepted, server) = tracked_response_server(vec![
        r#"{"catomic_replacement":"ONE\n"}"#.to_string(),
        r#"{"catomic_replacement":"TWO\n"}"#.to_string(),
    ]);
    settings.inline.block_mode = InlineBlockMode::Queued;
    let mut app =
        app_with(">> Rename\n<catblock>\none\n</catblock>\n<catblock>\ntwo\n</catblock>\n");
    let mut out = Vec::new();
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    poll_until_not_running(&mut app, &mut out);
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    wait_until(|| accepted.load(Ordering::SeqCst) == 2);
    app.handle_key_with(&mut out, key(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(!is_busy(&app));
    assert!(app.buffer.to_string().starts_with(">> Rename\n"));
    assert!(app.buffer.to_string().contains("ONE\n"));
    assert!(app.buffer.to_string().contains("two\n"));
    server.join().unwrap();
}

#[test]
fn queued_continue_on_error_is_serial_and_preserves_retry_instruction() {
    let (mut settings, _, _, server) = tracked_response_server(vec![
        "malformed".to_string(),
        r#"{"catomic_replacement":"TWO\n"}"#.to_string(),
    ]);
    settings.inline.block_mode = InlineBlockMode::Queued;
    settings.inline.stop_on_error = false;
    let mut app =
        app_with(">> Rename\n<catblock>\none\n</catblock>\n<catblock>\ntwo\n</catblock>\n");
    let mut out = Vec::new();
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    poll_until_not_running(&mut app, &mut out);
    assert!(matches!(app.inline_clanker.phase, Some(Phase::Preview(_))));
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert!(app.buffer.to_string().starts_with(">> Rename\n"));
    assert!(app.buffer.to_string().contains("one\n"));
    assert!(app.buffer.to_string().contains("TWO\n"));
    server.join().unwrap();
}

#[test]
fn full_file_warning_is_typed_one_shot_and_precedes_normal_confirmation() {
    let (mut inline_settings, _, accepted, server) = tracked_response_server(Vec::new());
    inline_settings.inline.warn_lines = 2;
    let mut app = app_with(">> Rewrite all\none\ntwo\n");
    let mut out = Vec::new();

    prepare::begin_with_settings(&mut app, &mut out, inline_settings).unwrap();
    assert!(matches!(app.inline_clanker.phase, Some(Phase::Warning(_))));
    assert!(app.message.as_deref().unwrap().contains("4 lines /"));
    assert!(app.message.as_deref().unwrap().contains("Type yes or no"));
    type_text(&mut app, &mut out, "maybe");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(app.inline_clanker.phase, Some(Phase::Warning(_))));
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("Please type yes or no"));
    type_text(&mut app, &mut out, "yes");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(app.inline_clanker.phase, Some(Phase::Confirm(_))));
    assert!(display_buffer(&app)
        .unwrap()
        .to_string()
        .contains("Enter sends; Esc cancels"));
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        0,
        "typed yes must not skip send confirmation"
    );
    app.handle_key_with(&mut out, key(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(!is_busy(&app));
    server.join().unwrap();

    let (mut inline_settings, _, accepted, server) = tracked_response_server(Vec::new());
    inline_settings.inline.warn_lines = 2;
    let mut app = app_with(">> Rewrite all\none\ntwo\n");
    prepare::begin_with_settings(&mut app, &mut out, inline_settings).unwrap();
    type_text(&mut app, &mut out, "no");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(!is_busy(&app));
    assert!(app.message.is_none());
    assert_eq!(accepted.load(Ordering::SeqCst), 0);
    server.join().unwrap();
}

#[test]
fn full_file_patch_preserves_sentinel_then_previews_cleanup() {
    let sentinel = "[[CATOMIC-INSTRUCTION-METADATA-1]]";
    let patch = format!(
        "--- a/untitled.txt\n+++ b/untitled.txt\n@@ -1,2 +1,2 @@\n {sentinel}\n-old\n+new\n"
    );
    let (settings, _, server) = response_server(vec![patch]);
    let mut app = app_with(">> Rewrite all\nold\n");
    let mut out = Vec::new();
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    poll_until_not_running(&mut app, &mut out);
    assert!(matches!(app.inline_clanker.phase, Some(Phase::Preview(_))));
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.buffer.to_string(), "new\n");
    app.handle_key_with(&mut out, key(KeyCode::Char('z'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.buffer.to_string(), ">> Rewrite all\nold\n");
    server.join().unwrap();
}

#[test]
fn disabled_cleanup_leaves_the_exact_instruction_line() {
    let (mut settings, _, server) =
        response_server(vec![r#"{"catomic_replacement":"CAT\n"}"#.to_string()]);
    settings.inline.remove_instruction_after_apply = false;
    let source = "  >> Rewrite cat  \n<catblock>\ncat\n</catblock>\n";
    let mut app = app_with(source);
    let mut out = Vec::new();
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    poll_until_not_running(&mut app, &mut out);
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(
        app.buffer.to_string(),
        "  >> Rewrite cat  \n<catblock>\nCAT\n</catblock>\n"
    );
    server.join().unwrap();
}

#[test]
fn cleanup_only_proposal_retains_a_semantic_deletion_gutter_and_undo_state() {
    let (settings, _, server) =
        response_server(vec![r#"{"catomic_replacement":"cat\n"}"#.to_string()]);
    let original = ">> Keep content\n<catblock>\ncat\n</catblock>\n";
    let mut app = app_with(original);
    let mut out = Vec::new();
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    poll_until_not_running(&mut app, &mut out);
    assert!(matches!(app.inline_clanker.phase, Some(Phase::Preview(_))));
    out.clear();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "<catblock>\ncat\n</catblock>\n");
    assert!(source_changes(&app).is_some());
    assert!(String::from_utf8_lossy(&out).contains('┃'));

    app.handle_key_with(&mut out, key(KeyCode::Char('z'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.buffer.to_string(), original);
    assert!(source_changes(&app).is_none());
    server.join().unwrap();
}

#[test]
fn crlf_source_cleanup_is_undoable_and_never_auto_saves() {
    let path = temp_file(
        "inline_crlf",
        b">> Rewrite\r\n<catblock>\r\ncat\r\n</catblock>\r\n",
    );
    let (settings, _, server) =
        response_server(vec![r#"{"catomic_replacement":"CAT\n"}"#.to_string()]);
    let mut app = super::super::App::new(path.to_str()).unwrap();
    let mut out = Vec::new();
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    poll_until_not_running(&mut app, &mut out);
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "<catblock>\nCAT\n</catblock>\n");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b">> Rewrite\r\n<catblock>\r\ncat\r\n</catblock>\r\n"
    );
    app.handle_key_with(&mut out, key(KeyCode::Char('z'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(
        app.buffer.to_string(),
        ">> Rewrite\n<catblock>\ncat\n</catblock>\n"
    );
    server.join().unwrap();
    std::fs::remove_file(path).unwrap();
}

#[test]
fn drift_before_send_or_apply_fails_closed_and_keeps_instruction() {
    let (settings, _, accepted, server) = tracked_response_server(Vec::new());
    let mut app = app_with(">> Rewrite\n<catblock>\ncat\n</catblock>\n");
    let mut out = Vec::new();
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();
    app.buffer.set_cursor(Cursor { row: 2, col: 3 });
    app.buffer.insert_char('!');
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(!is_busy(&app));
    assert!(app.buffer.to_string().starts_with(">> Rewrite\n"));
    assert_eq!(accepted.load(Ordering::SeqCst), 0);
    server.join().unwrap();

    let (settings, _, server) =
        response_server(vec![r#"{"catomic_replacement":"CAT\n"}"#.to_string()]);
    let mut app = app_with(">> Rewrite\n<catblock>\ncat\n</catblock>\n");
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    poll_until_not_running(&mut app, &mut out);
    app.buffer.set_cursor(Cursor { row: 0, col: 0 });
    app.buffer.insert_char('!');
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(app.message.as_deref().unwrap().contains("drifted"));
    assert!(app.buffer.to_string().contains(">> Rewrite"));
    server.join().unwrap();
}

#[test]
fn exact_selection_and_delimiter_guards_detect_drift_independently_of_revision() {
    let mut app = app_with(">> Uppercase\nPRIVATE\npublic\nafter\n");
    app.buffer.set_cursor(Cursor { row: 2, col: 0 });
    let mut out = Vec::new();
    for _ in 0..6 {
        app.handle_key_with(&mut out, key(KeyCode::Right, KeyModifiers::SHIFT))
            .unwrap();
    }
    prepare::begin_with_settings(&mut app, &mut out, LlmSettings::default()).unwrap();
    let Some(Phase::Confirm(mut confirmation)) = app.inline_clanker.phase.take() else {
        panic!("selection should reach confirmation")
    };
    app.buffer.set_cursor(Cursor { row: 2, col: 3 });
    app.buffer.insert_char('!');
    confirmation.prepared.expected_revision = app.buffer.edit_history_position();
    assert_eq!(
        request::validate_identity(&app, &confirmation.prepared),
        Err("a captured edit target drifted")
    );

    let mut app = app_with(">> Rewrite\n<catblock>\ncat\n</catblock>\n");
    prepare::begin_with_settings(&mut app, &mut out, LlmSettings::default()).unwrap();
    let Some(Phase::Confirm(mut confirmation)) = app.inline_clanker.phase.take() else {
        panic!("block should reach confirmation")
    };
    app.buffer.set_cursor(Cursor { row: 1, col: 0 });
    app.buffer.insert_char('!');
    confirmation.prepared.expected_revision = app.buffer.edit_history_position();
    assert_eq!(
        request::validate_identity(&app, &confirmation.prepared),
        Err("a context delimiter drifted")
    );
}

#[test]
fn non_color_fallback_and_clear_action_are_render_only() {
    let (settings, _, server) =
        response_server(vec![r#"{"catomic_replacement":"CAT\n"}"#.to_string()]);
    let mut app = app_with(">> Rewrite\n<catblock>\ncat\n</catblock>\n");
    app.theme = crate::config::theme::parse("[theme]\nname = 'mono'\n").unwrap();
    app.view.soft_wrap = true;
    app.screen.width = 8;
    let mut out = Vec::new();
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    poll_until_not_running(&mut app, &mut out);
    out.clear();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    let bytes = app.buffer.to_string();
    let rendered = String::from_utf8_lossy(&out);
    assert!(rendered.contains("\x1b[4;7m"), "got {rendered:?}");
    assert!(rendered.contains("\x1b[1;4;7m!\x1b[0m"));
    app.handle_resize(12, 6, &mut out).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    assert!(source_changes(&app).is_some());

    app.handle_key_with(&mut out, key(KeyCode::F(3), KeyModifiers::SHIFT))
        .unwrap();
    assert!(source_changes(&app).is_none());
    assert_eq!(app.buffer.to_string(), bytes);
    server.join().unwrap();
}

#[test]
fn running_request_blocks_edits_paste_and_competing_surfaces_until_cancelled() {
    let (settings, _, accepted, server) = tracked_delayed_response_server(vec![(
        Duration::from_millis(300),
        r#"{"catomic_replacement":"CAT\n"}"#.to_string(),
    )]);
    let source = ">> Rewrite\n<catblock>\ncat\n</catblock>\n";
    let mut app = app_with(source);
    let mut out = Vec::new();
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    wait_until(|| accepted.load(Ordering::SeqCst) == 1);

    app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    super::super::input::handle_paste(&mut app, &mut out, "PASTE").unwrap();
    app.handle_key_with(&mut out, key(KeyCode::F(10), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.buffer.to_string(), source);
    assert!(matches!(app.inline_clanker.phase, Some(Phase::Running(_))));
    assert!(!super::super::model_picker::is_viewing(&app));
    assert!(app.message.as_deref().unwrap().contains("input is paused"));

    app.handle_key_with(&mut out, key(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(!is_busy(&app));
    assert_eq!(app.buffer.to_string(), source);
    server.join().unwrap();
}

#[test]
fn process_local_backend_selection_drives_inline_requests() {
    let (mut settings, requests, server) =
        response_server(vec![r#"{"catomic_replacement":"SELECTED\n"}"#.to_string()]);
    let mut selected = settings.default_preset().clone();
    selected.name = "selected command".to_string();
    selected.model = "selected-model".to_string();
    let mut unavailable = selected.clone();
    unavailable.name = "default unavailable".to_string();
    unavailable.model = "wrong-model".to_string();
    let crate::config::llm::BackendAdapter::Command(command) = &mut unavailable.adapter else {
        unreachable!("test fixture is a command backend")
    };
    command.program = "/catomic/missing-inline-default".to_string();
    settings.default = unavailable.name.clone();
    settings.presets = vec![unavailable, selected.clone()];

    let mut app = app_with(">> Rewrite\n<catblock>\nold\n</catblock>\n");
    app.model_session.select(selected);
    let mut out = Vec::new();
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();
    assert!(display_buffer(&app)
        .unwrap()
        .to_string()
        .contains("Model: selected-model"));
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    poll_until_not_running(&mut app, &mut out);
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert!(app.buffer.to_string().contains("SELECTED\n"));
    server.join().unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[test]
fn hard_limit_cannot_be_bypassed_by_the_soft_warning() {
    let mut app = app_with(&format!(">> Rewrite all\n{}", "line\n".repeat(2_000)));
    let mut settings = LlmSettings::default();
    settings.inline.warn_lines = 1;
    let mut out = Vec::new();
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();

    assert!(!is_busy(&app));
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("limit is 2000 lines"));
}

#[test]
fn switching_buffers_cancels_queue_state_without_moving_highlights() {
    let first = temp_file(
        "inline_buffer_first",
        b">> Rewrite\n<catblock>\none\n</catblock>\n",
    );
    let second = temp_file("inline_buffer_second", b"second\n");
    let paths = vec![
        first.to_string_lossy().into_owned(),
        second.to_string_lossy().into_owned(),
    ];
    let mut app = super::super::App::new_with_paths_and_big_file_config(
        &paths,
        crate::config::big_files::BigFileConfig::default(),
    )
    .unwrap();
    let mut out = Vec::new();
    prepare::begin_with_settings(&mut app, &mut out, LlmSettings::default()).unwrap();
    assert!(is_busy(&app));

    assert!(app.switch_buffer(super::super::buffers::BufferDirection::Next));
    assert!(!is_busy(&app));
    assert_eq!(app.buffer.to_string(), "second\n");

    std::fs::remove_file(first).unwrap();
    std::fs::remove_file(second).unwrap();
}

#[test]
fn applied_change_marks_remain_owned_by_their_buffer() {
    let first = temp_file(
        "inline_highlight_first",
        b">> Rewrite\n<catblock>\ncat\n</catblock>\n",
    );
    let second = temp_file("inline_highlight_second", b"second\n");
    let paths = vec![
        first.to_string_lossy().into_owned(),
        second.to_string_lossy().into_owned(),
    ];
    let (settings, _, server) =
        response_server(vec![r#"{"catomic_replacement":"CAT\n"}"#.to_string()]);
    let mut app = super::super::App::new_with_paths_and_big_file_config(
        &paths,
        crate::config::big_files::BigFileConfig::default(),
    )
    .unwrap();
    let mut out = Vec::new();
    prepare::begin_with_settings(&mut app, &mut out, settings).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    poll_until_not_running(&mut app, &mut out);
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(source_changes(&app).is_some());

    assert!(app.switch_buffer(super::super::buffers::BufferDirection::Next));
    assert!(source_changes(&app).is_none());
    assert_eq!(app.buffer.to_string(), "second\n");
    assert!(app.switch_buffer(super::super::buffers::BufferDirection::Previous));
    assert!(source_changes(&app).is_some());
    assert!(app.buffer.to_string().contains("CAT\n"));
    assert_eq!(
        std::fs::read_to_string(&first).unwrap(),
        ">> Rewrite\n<catblock>\ncat\n</catblock>\n"
    );

    server.join().unwrap();
    std::fs::remove_file(first).unwrap();
    std::fs::remove_file(second).unwrap();
}
