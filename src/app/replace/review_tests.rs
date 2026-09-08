//! End-to-end semantic input regressions for selective replacement.

use super::*;
use crate::app::App;

fn key(app: &mut App, out: &mut Vec<u8>, code: KeyCode) {
    app.handle_key_with(out, KeyEvent::new(code, KeyModifiers::NONE))
        .unwrap();
}

fn start(app: &mut App, out: &mut Vec<u8>, find: &str, replacement: &str) {
    app.handle_key_with(
        out,
        KeyEvent::new(
            KeyCode::Char('f'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
    )
    .unwrap();
    crate::app::input::handle_paste(app, out, find).unwrap();
    key(app, out, KeyCode::Enter);
    crate::app::input::handle_paste(app, out, replacement).unwrap();
    key(app, out, KeyCode::Enter);
}

fn app(text: &str) -> App {
    let mut app = App::new(None).unwrap();
    app.buffer = Box::new(crate::buffer::PieceTable::from_text(text));
    app
}

#[test]
fn accepted_replacement_leaves_joined_graphemes_behind_the_cursor() {
    for (original, replacement, joined, end_col) in [
        ("🇫x🇷!", "", "🇫🇷!", 2),
        ("👩x💻!", "\u{200d}", "👩\u{200d}💻!", 3),
    ] {
        for action in ['y', 'a'] {
            let mut app = app(original);
            let mut out = Vec::new();
            start(&mut app, &mut out, "x", replacement);
            key(&mut app, &mut out, KeyCode::Char(action));
            poll(&mut app, &mut out).unwrap();
            assert!(!is_active(&app));
            assert_eq!(app.buffer.to_string(), joined);
            assert_eq!(
                app.buffer.cursor(),
                Cursor {
                    row: 0,
                    col: end_col
                }
            );

            key(&mut app, &mut out, KeyCode::Backspace);
            assert_eq!(app.buffer.to_string(), "!");
            for expected in [joined, original] {
                app.handle_key_with(
                    &mut out,
                    KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
                )
                .unwrap();
                assert_eq!(app.buffer.to_string(), expected);
            }
            for expected in [joined, "!"] {
                app.handle_key_with(
                    &mut out,
                    KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
                )
                .unwrap();
                assert_eq!(app.buffer.to_string(), expected);
            }
        }
    }
}

#[test]
fn review_waits_for_explicit_accept_then_skips_and_cancels_without_retyping() {
    let mut app = app("cat cat cat");
    let mut out = Vec::new();
    let history = app.buffer.edit_history_position();
    start(&mut app, &mut out, "cat", "fox");
    assert_eq!(
        app.buffer.to_string(),
        "cat cat cat",
        "second Enter must review, not replace"
    );
    assert_eq!(app.buffer.edit_history_position(), history);
    assert!(!app.file.dirty);
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("Replace candidate"));
    key(&mut app, &mut out, KeyCode::Char('n'));
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 4 });
    key(&mut app, &mut out, KeyCode::Char('y'));
    assert_eq!(app.buffer.to_string(), "cat fox cat");
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 8 });
    key(&mut app, &mut out, KeyCode::Esc);
    assert!(!is_active(&app));
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("1 replaced, 1 skipped"));
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "cat cat cat");
}

#[test]
fn review_wraps_once_and_never_revisits_replacement_containing_the_query() {
    let mut app = app("aa aa aa");
    app.buffer.set_cursor(Cursor { row: 0, col: 4 });
    let mut out = Vec::new();
    start(&mut app, &mut out, "aa", "aaaa");
    assert_eq!(app.buffer.to_string(), "aa aa aa");
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 6 });
    key(&mut app, &mut out, KeyCode::Char('y'));
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 0 });
    key(&mut app, &mut out, KeyCode::Char('n'));
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 3 });
    key(&mut app, &mut out, KeyCode::Char('a'));
    assert!(!is_active(&app));
    assert_eq!(app.buffer.to_string(), "aa aaaa aaaa");
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("2 replaced, 1 skipped"));
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "aa aa aaaa");
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "aa aa aa");
}

#[test]
fn skip_preserves_overlapping_candidates_and_accept_advances_past_inserted_text() {
    let mut app = app("aaaaa");
    let mut out = Vec::new();
    start(&mut app, &mut out, "aaa", "x");
    assert_eq!(app.buffer.to_string(), "aaaaa");
    key(&mut app, &mut out, KeyCode::Char('n'));
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 1 });
    key(&mut app, &mut out, KeyCode::Char('y'));
    assert_eq!(app.buffer.to_string(), "axa");
    assert!(!is_active(&app));
}

#[test]
fn review_paste_and_unrelated_typing_never_edit_the_source() {
    let mut app = app("猫 a\u{301} 猫");
    let mut out = Vec::new();
    start(&mut app, &mut out, "猫", "👩\u{200d}💻\n");
    assert_eq!(app.buffer.to_string(), "猫 a\u{301} 猫");
    crate::app::input::handle_paste(&mut app, &mut out, "yna\n").unwrap();
    key(&mut app, &mut out, KeyCode::Char('x'));
    key(&mut app, &mut out, KeyCode::Delete);
    assert_eq!(app.buffer.to_string(), "猫 a\u{301} 猫");
    key(&mut app, &mut out, KeyCode::Char('y'));
    assert_eq!(app.buffer.to_string(), "👩\u{200d}💻\n a\u{301} 猫");
    assert_eq!(app.buffer.cursor(), Cursor { row: 1, col: 4 });
    key(&mut app, &mut out, KeyCode::Esc);
    assert!(!is_active(&app));
}

#[test]
fn remaining_all_is_bounded_cancellable_and_each_bulk_group_undoes_together() {
    let original = "cat ".repeat(5000);
    let mut app = app(&original);
    let mut out = Vec::new();
    start(&mut app, &mut out, "cat", "dog");
    key(&mut app, &mut out, KeyCode::Char('y'));
    assert_eq!(app.buffer.to_string().matches("dog").count(), 1);
    key(&mut app, &mut out, KeyCode::Char('a'));
    let accepted = app.buffer.to_string().matches("dog").count();
    assert_eq!(
        accepted, 129,
        "one bounded bulk group follows the manual replacement"
    );
    assert!(is_running(&app));
    key(&mut app, &mut out, KeyCode::Esc);
    assert!(!is_active(&app));
    poll(&mut app, &mut out).unwrap();
    assert_eq!(app.buffer.to_string().matches("dog").count(), accepted);
    app.buffer.undo();
    assert_eq!(app.buffer.to_string().matches("dog").count(), 1);
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), original);
}

#[test]
fn large_absent_search_yields_and_can_cancel_before_scanning_the_whole_buffer() {
    let original = "x".repeat(1024 * 1024);
    let mut app = app(&original);
    let mut out = Vec::new();
    start(&mut app, &mut out, "needle", "replacement");
    assert!(is_running(&app));
    assert!(active_match(&app).is_none());
    key(&mut app, &mut out, KeyCode::Esc);
    poll(&mut app, &mut out).unwrap();
    assert!(!is_active(&app));
    assert_eq!(app.buffer.to_string(), original);
    assert!(!app.file.dirty);
}

#[test]
fn changed_source_cannot_apply_a_stale_candidate() {
    let mut app = app("cat cat");
    let mut out = Vec::new();
    start(&mut app, &mut out, "cat", "dog");
    app.buffer
        .replace_range(Cursor::default(), Cursor { row: 0, col: 3 }, "NEW")
        .unwrap();
    assert!(active_match(&app).is_none());
    key(&mut app, &mut out, KeyCode::Char('y'));
    assert!(!is_active(&app));
    assert_eq!(app.buffer.to_string(), "NEW cat");
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("source changed"));
}

#[test]
fn review_actions_can_be_remapped_and_unbound_without_typing_fallthrough() {
    let mut app = app("cat cat");
    let mut out = Vec::new();
    app.keybindings = crate::config::keybindings::parse(
        r#"
        [keybindings]
        replace-accept = ["alt+y"]
        replace-skip = ["s"]
        replace-remaining = []
        replace-cancel = ["alt+c"]
    "#,
    )
    .unwrap();
    start(&mut app, &mut out, "cat", "dog");
    for code in [
        KeyCode::Char('y'),
        KeyCode::Char('n'),
        KeyCode::Char('a'),
        KeyCode::Esc,
        KeyCode::Enter,
    ] {
        key(&mut app, &mut out, code);
    }
    assert_eq!(app.buffer.to_string(), "cat cat");
    assert!(is_active(&app));
    key(&mut app, &mut out, KeyCode::Char('s'));
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 4 });
    app.handle_key_with(
        &mut out,
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT),
    )
    .unwrap();
    assert_eq!(app.buffer.to_string(), "cat dog");
    assert!(!is_active(&app));
    start(&mut app, &mut out, "dog", "fox");
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("Alt+Y replace"));
    app.handle_key_with(
        &mut out,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT),
    )
    .unwrap();
    assert_eq!(app.buffer.to_string(), "cat dog");
    assert!(!is_active(&app));
}

#[test]
fn identical_replacement_terminates_without_dirtying_or_recording_empty_undo() {
    let mut app = app("aa aa");
    let mut out = Vec::new();
    let history = app.buffer.edit_history_position();
    start(&mut app, &mut out, "aa", "aa");
    key(&mut app, &mut out, KeyCode::Char('a'));
    assert!(!is_active(&app));
    assert_eq!(app.buffer.edit_history_position(), history);
    assert!(!app.file.dirty);
}

#[test]
fn empty_replacement_at_eof_and_initial_origin_inside_match_complete_once() {
    for (text, origin, query, expected) in [("aa", 1, "aa", ""), ("猫\n猫", 0, "猫", "\n")] {
        let mut app = app(text);
        app.buffer.set_cursor(Cursor {
            row: 0,
            col: origin,
        });
        let mut out = Vec::new();
        start(&mut app, &mut out, query, "");
        key(&mut app, &mut out, KeyCode::Char('a'));
        assert!(!is_active(&app));
        assert_eq!(app.buffer.to_string(), expected);
        assert!(active_match(&app).is_none());
        assert!(!crate::app::search::is_active(&app));
        assert!(!crate::app::completion::is_active(&app));
    }
}

#[test]
fn paged_replacement_is_refused_without_mutating_the_visible_page() {
    let path =
        std::env::temp_dir().join(format!("catomic_review_paged_{}.txt", std::process::id()));
    std::fs::write(&path, "cat\ncat\n").unwrap();
    let mut app = app("");
    app.buffer = Box::new(crate::buffer::PagedFileBuffer::open(&path, 1).unwrap());
    let mut out = Vec::new();
    start(&mut app, &mut out, "cat", "dog");
    assert!(!is_active(&app));
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("unavailable for paged files"));
    assert_eq!(app.buffer.line(0).as_deref(), Some("cat"));
    assert!(!app.file.dirty);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn source_clicks_and_prior_search_completion_do_not_disrupt_review() {
    let mut app = app("cat cat");
    let mut out = Vec::new();
    crate::app::search::open_prompt(&mut app, &mut out).unwrap();
    crate::app::search::handle_paste(&mut app, &mut out, "cat").unwrap();
    open_prompt(&mut app, &mut out, false).unwrap();
    assert!(!crate::app::search::is_active(&app));
    crate::app::input::handle_paste(&mut app, &mut out, "cat").unwrap();
    key(&mut app, &mut out, KeyCode::Enter);
    crate::app::input::handle_paste(&mut app, &mut out, "dog").unwrap();
    key(&mut app, &mut out, KeyCode::Enter);
    crate::app::selection::handle_mouse(
        &mut app,
        &mut out,
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 5,
            row: 0,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();
    assert_eq!(app.buffer.cursor(), Cursor::default());
    assert!(app.selection.active().is_none());
    key(&mut app, &mut out, KeyCode::Esc);
    assert!(active_match(&app).is_none());
    assert!(!crate::app::completion::is_active(&app));
}

#[test]
fn wrapping_does_not_offer_a_match_that_overlaps_an_accepted_insertion() {
    for (text, replacement, expected) in [("aaa", "a", "aa"), ("aaaa", "aaa", "aaaaa")] {
        let mut app = app(text);
        app.buffer.set_cursor(Cursor { row: 0, col: 1 });
        let mut out = Vec::new();
        start(&mut app, &mut out, "aa", replacement);
        key(&mut app, &mut out, KeyCode::Char('y'));
        assert_eq!(app.buffer.to_string(), expected);
        assert!(
            !is_active(&app),
            "wrapped matches must not include inserted text"
        );
        assert!(app.message.as_deref().unwrap_or("").contains("1 replaced"));
    }
}

#[test]
fn all_after_accept_wraps_only_through_unhandled_nonoverlapping_text() {
    for (replacement, expected) in [("a", "aaa"), ("aaa", "aaaaaaaaa")] {
        let mut app = app("aaaaaa");
        app.buffer.set_cursor(Cursor { row: 0, col: 2 });
        let mut out = Vec::new();
        start(&mut app, &mut out, "aa", replacement);
        key(&mut app, &mut out, KeyCode::Char('y'));
        key(&mut app, &mut out, KeyCode::Char('a'));
        for _ in 0..3 {
            poll(&mut app, &mut out).unwrap();
        }
        assert!(!is_active(&app));
        assert_eq!(app.buffer.to_string(), expected);
        assert!(app.message.as_deref().unwrap_or("").contains("3 replaced"));
    }
}

#[test]
fn dirty_quit_confirmation_pauses_bulk_until_explicit_review_action() {
    let mut app = app(&"cat ".repeat(5000));
    let mut out = Vec::new();
    start(&mut app, &mut out, "cat", "dog");
    key(&mut app, &mut out, KeyCode::Char('a'));
    let accepted = app.buffer.to_string().matches("dog").count();
    app.handle_key_with(
        &mut out,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert!(app.pending_quit_confirm);
    assert!(!is_running(&app));
    poll(&mut app, &mut out).unwrap();
    assert_eq!(app.buffer.to_string().matches("dog").count(), accepted);
    key(&mut app, &mut out, KeyCode::Esc);
    assert!(!app.pending_quit_confirm);
    assert!(!is_active(&app));
    assert_eq!(app.buffer.to_string().matches("dog").count(), accepted);
}

#[test]
fn large_replacement_text_limits_bulk_group_bytes_as_well_as_match_count() {
    let mut app = app(&"cat ".repeat(100));
    let replacement = "x".repeat(8192);
    let mut out = Vec::new();
    start(&mut app, &mut out, "cat", &replacement);
    key(&mut app, &mut out, KeyCode::Char('a'));
    assert_eq!(
        app.buffer.to_string().matches('x').count(),
        7 * replacement.len()
    );
    assert!(is_running(&app));
    key(&mut app, &mut out, KeyCode::Esc);
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "cat ".repeat(100));
}
