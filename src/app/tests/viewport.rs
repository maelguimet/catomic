//! App viewport/scroll/resize/reveal tests (child submodule of app::tests).
//!
//! Purpose: this file must contain the App-level tests for viewport behavior
//! (scroll_top/scroll_left, reveal_cursor, resize, zero-size handling, horiz/vert clamp).
//! Owns: all viewport/scroll/resize/reveal/clamp related #[test] functions.
//! Must not: contain runtime logic or be included outside test builds.
//! Invariants: loaded as mod viewport; from tests.rs under #[cfg(test)] mod tests;
//!              uses `use super::super::*;` to reach private App methods and `use super::make_key;`
//!              for the shared key helper (kept pub(super) in hub).

use super::super::*;
use super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};

#[test]
fn app_new_has_default_screen_size_and_scroll() {
    let app = App::new(None).unwrap();
    assert_eq!(app.screen.width, 80, "default width");
    assert_eq!(
        app.screen.height, 24,
        "default height (matches prior hardcoded)"
    );
    assert_eq!(app.screen.scroll_top, 0);
}

#[test]
fn app_render_respects_screen_height_via_captured_writer() {
    let mut app = App::new(None).unwrap();
    app.screen.height = 10;
    app.screen.scroll_top = 0;

    let mut out: Vec<u8> = Vec::new();
    app.handle_key_with(&mut out, make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    let rendered = String::from_utf8_lossy(&out);
    assert!(
        rendered.contains("\x1b[10;1H"),
        "render must use screen height for bottom row positioning"
    );
    assert!(rendered.contains("\x1b[K"), "clears using \\x1b[K");
}

#[test]
fn app_handle_resize_updates_screen_and_renders() {
    let mut app = App::new(None).unwrap();
    assert_eq!(app.screen.height, 24);

    let mut out: Vec<u8> = Vec::new();
    app.handle_resize(50, 15, &mut out).unwrap();

    assert_eq!(app.screen.width, 50);
    assert_eq!(app.screen.height, 15);
    let rendered = String::from_utf8_lossy(&out);
    assert!(
        rendered.contains("\x1b[15;1H"),
        "resize render must position using new screen height"
    );
    assert!(!out.is_empty(), "resize must have triggered a render");
}

#[test]
fn foreground_redraw_preserves_unsaved_buffer_and_active_prompt() {
    let mut app = App::new(None).unwrap();
    let mut out = Vec::new();
    app.handle_key_with(&mut out, make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    super::super::command_prompt::open_command_prompt(&mut app, &mut out).unwrap();
    let history = app.buffer.edit_history_position();

    out.clear();
    super::super::viewport::redraw_after_focus(&mut app, Some((31, 9)), &mut out).unwrap();

    assert_eq!(app.buffer.to_string(), "x");
    assert_eq!(app.buffer.edit_history_position(), history);
    assert!(super::super::command_prompt::is_active(&app));
    assert_eq!(app.message.as_deref(), Some("Command: "));
    assert_eq!((app.screen.width, app.screen.height), (31, 9));
    assert!(!out.is_empty());
}

#[test]
fn app_cursor_down_past_visible_updates_scroll_top() {
    let mut app = App::new(None).unwrap();
    app.screen.height = 6;
    app.screen.scroll_top = 0;

    let mut sink: Vec<u8> = Vec::new();
    for _ in 0..9 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
    }
    assert_eq!(app.buffer.cursor().row, 9);

    assert!(
        app.screen.scroll_top >= 6,
        "down past viewport must update scroll_top; got {}",
        app.screen.scroll_top
    );
}

#[test]
fn app_render_after_reveal_omits_earlier_lines_and_shows_cursor_row() {
    let mut app = App::new(None).unwrap();
    app.screen.height = 6;
    app.screen.scroll_top = 0;

    for i in 0..10 {
        if i > 0 {
            app.buffer.insert_newline();
        }
        app.buffer.insert_char('L');
        let marker = char::from(b'0' + (i % 10) as u8);
        app.buffer.insert_char(marker);
    }
    assert_eq!(app.buffer.cursor().row, 9);

    let mut sink: Vec<u8> = Vec::new();
    app.screen.scroll_top = 0;
    for _ in 0..9 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Up, KeyModifiers::NONE))
            .unwrap();
    }
    assert_eq!(app.buffer.cursor().row, 0);
    app.screen.scroll_top = 0;

    for _ in 0..9 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Down, KeyModifiers::NONE))
            .unwrap();
    }
    assert_eq!(app.buffer.cursor().row, 9);
    assert!(
        app.screen.scroll_top > 0,
        "must have scrolled; scroll_top={}",
        app.screen.scroll_top
    );

    let mut out: Vec<u8> = Vec::new();
    app.render(&mut out).unwrap();
    let rendered = String::from_utf8_lossy(&out);

    assert!(
        !rendered.contains("L0"),
        "early line content must not be emitted when scrolled; scroll_top={}\nout: {}",
        app.screen.scroll_top,
        rendered
    );
    assert!(
        rendered.contains("L9"),
        "cursor row content must be emitted; got scroll_top={} rendered=\n{}",
        app.screen.scroll_top,
        rendered
    );
}

#[test]
fn app_resize_smaller_reveals_cursor_row() {
    let mut app = App::new(None).unwrap();
    for _ in 0..15 {
        app.buffer.insert_newline();
    }
    assert_eq!(app.buffer.cursor().row, 15);
    app.screen.height = 30;
    app.screen.scroll_top = 0;

    let mut out: Vec<u8> = Vec::new();
    app.handle_resize(40, 10, &mut out).unwrap();

    assert_eq!(app.screen.height, 10);
    assert!(
        app.screen.scroll_top > 0,
        "resize to smaller must reveal; scroll_top={}",
        app.screen.scroll_top
    );
    let vh = app.screen.visible_height();
    assert!(
        app.screen.scroll_top <= 15 && 15 < app.screen.scroll_top + vh,
        "cursor row 15 must be visible after small resize; scroll_top={}, vh={}",
        app.screen.scroll_top,
        vh
    );
    assert!(!out.is_empty(), "resize must render");
}

#[test]
fn app_typing_past_visible_width_updates_scroll_left() {
    let mut app = App::new(None).unwrap();
    app.screen.width = 5;
    app.screen.height = 6;
    app.screen.scroll_left = 0;

    let mut sink: Vec<u8> = Vec::new();
    for _ in 0..8 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Char('x'), KeyModifiers::NONE))
            .unwrap();
    }
    assert_eq!(app.buffer.cursor().col, 8);
    assert!(
        app.screen.scroll_left >= 4,
        "typing past width must update scroll_left; got {}",
        app.screen.scroll_left
    );
}

#[test]
fn app_render_after_horizontal_reveal_omits_earlier_chars_and_shows_cursor_side() {
    let mut app = App::new(None).unwrap();
    app.screen.width = 6;
    app.screen.height = 4;
    app.screen.scroll_left = 0;

    for c in "0123456789ABCDEF".chars() {
        app.buffer.insert_char(c);
    }
    assert_eq!(app.buffer.cursor().col, 16);

    let mut sink: Vec<u8> = Vec::new();
    for _ in 0..5 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Left, KeyModifiers::NONE))
            .unwrap();
    }
    for _ in 0..10 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Right, KeyModifiers::NONE))
            .unwrap();
    }
    assert!(
        app.screen.scroll_left > 0,
        "must have scrolled horizontally; scroll_left={}",
        app.screen.scroll_left
    );

    let mut out: Vec<u8> = Vec::new();
    app.render(&mut out).unwrap();
    let rendered = String::from_utf8_lossy(&out);

    assert!(
        !rendered.contains("01"),
        "early line chars must be omitted after horiz scroll; scroll_left={}\n{}",
        app.screen.scroll_left,
        rendered
    );
    let has_late = rendered.contains("A")
        || rendered.contains("B")
        || rendered.contains("C")
        || rendered.contains("D")
        || rendered.contains("E")
        || rendered.contains("F");
    assert!(
        has_late,
        "cursor-side content should be rendered; scroll_left={}\n{}",
        app.screen.scroll_left, rendered
    );
}

#[test]
fn app_resize_narrower_reveals_current_cursor_column() {
    let mut app = App::new(None).unwrap();
    for c in "ABCDEFGHIJKLMNOP".chars() {
        app.buffer.insert_char(c);
    }
    assert_eq!(app.buffer.cursor().col, 16);
    app.screen.width = 30;
    app.screen.scroll_left = 0;

    let mut out: Vec<u8> = Vec::new();
    app.handle_resize(5, 10, &mut out).unwrap();

    assert_eq!(app.screen.width, 5);
    assert!(
        app.screen.scroll_left > 0,
        "narrow resize must reveal cursor col; scroll_left={}",
        app.screen.scroll_left
    );
    let vw = app.screen.visible_width();
    assert!(
        app.screen.scroll_left <= 16 && 16 < app.screen.scroll_left + vw,
        "cursor col 16 must be visible after narrow resize; scroll_left={}, vw={}",
        app.screen.scroll_left,
        vw
    );
    assert!(!out.is_empty(), "resize must render");
}

#[test]
fn app_resize_to_zero_size_clamps_scroll_and_does_not_panic() {
    let mut app = App::new(None).unwrap();
    app.screen.scroll_top = 5;
    app.screen.scroll_left = 12;
    app.screen.width = 20;
    app.screen.height = 10;

    let mut out: Vec<u8> = Vec::new();
    app.handle_resize(0, 0, &mut out).unwrap();

    assert_eq!(app.screen.width, 0);
    assert_eq!(app.screen.height, 0);
    assert_eq!(
        app.screen.scroll_top, 0,
        "zero height resize must clamp scroll_top"
    );
    assert_eq!(
        app.screen.scroll_left, 0,
        "zero width resize must clamp scroll_left"
    );
    assert!(!out.is_empty());
}

#[test]
fn app_resize_to_zero_then_back_to_nonzero_typing_and_move_reveal_normally() {
    let mut app = App::new(None).unwrap();
    app.screen.width = 8;
    app.screen.height = 6;
    app.screen.scroll_left = 0;
    app.screen.scroll_top = 0;

    let mut sink: Vec<u8> = Vec::new();
    app.handle_resize(0, 0, &mut sink).unwrap();
    assert_eq!(app.screen.scroll_top, 0);
    assert_eq!(app.screen.scroll_left, 0);

    app.handle_resize(10, 8, &mut sink).unwrap();
    assert_eq!(app.screen.width, 10);
    assert_eq!(app.screen.height, 8);

    for _ in 0..12 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Char('x'), KeyModifiers::NONE))
            .unwrap();
    }
    assert!(
        app.screen.scroll_left > 0,
        "should have horiz scrolled while typing"
    );
    for _ in 0..8 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Left, KeyModifiers::NONE))
            .unwrap();
    }
    assert!(
        app.screen.scroll_left < 5,
        "moving left after horiz scroll should reduce scroll_left; got {}",
        app.screen.scroll_left
    );

    app.handle_key_with(&mut sink, make_key(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(&mut sink, make_key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(app.screen.scroll_top <= app.buffer.cursor().row);
}

#[test]
fn app_delete_and_backspace_after_horiz_scroll_reduce_scroll_left_when_cursor_before_viewport() {
    let mut app = App::new(None).unwrap();
    app.screen.width = 5;
    app.screen.height = 4;
    app.screen.scroll_left = 0;

    let mut sink: Vec<u8> = Vec::new();
    for c in "ABCDEFGHIJKLMNOPQRST".chars() {
        app.buffer.insert_char(c);
    }
    for _ in 0..20 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Left, KeyModifiers::NONE))
            .unwrap();
    }
    for _ in 0..15 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Right, KeyModifiers::NONE))
            .unwrap();
    }
    let initial_sl = app.screen.scroll_left;
    assert!(
        initial_sl > 0,
        "need horiz scroll established; scroll_left={}",
        initial_sl
    );

    let mut last_sl = app.screen.scroll_left;
    for _ in 0..10 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Backspace, KeyModifiers::NONE))
            .unwrap();
        if app.buffer.cursor().col < last_sl {
            assert!(
                app.screen.scroll_left <= last_sl,
                "backspace moving cursor before viewport should not increase scroll_left"
            );
        }
        last_sl = app.screen.scroll_left;
    }

    app.screen.scroll_left = 8;
    app.buffer.move_right();
    let pre = app.screen.scroll_left;
    app.handle_key_with(&mut sink, make_key(KeyCode::Delete, KeyModifiers::NONE))
        .unwrap();
    assert!(app.screen.scroll_left <= pre + 1);
}

#[test]
fn app_viewport_clamp_scroll_top_to_zero_when_buffer_shorter_than_viewport() {
    let mut app = App::new(None).unwrap();
    app.buffer.insert_newline();
    app.buffer.insert_newline();
    assert_eq!(app.buffer.line_count(), 3);

    app.screen.height = 11;
    app.screen.scroll_top = 5;

    app.reveal_cursor();

    assert_eq!(
        app.screen.scroll_top, 0,
        "buffer shorter than viewport must clamp scroll_top to 0"
    );
}

#[test]
fn app_viewport_clamps_scroll_top_after_manual_push_then_resize_or_reveal() {
    let mut app = App::new(None).unwrap();
    for _ in 0..19 {
        app.buffer.insert_newline();
    }
    assert_eq!(app.buffer.cursor().row, 19);
    let lc = app.buffer.line_count();
    assert_eq!(lc, 20);

    app.screen.height = 6;
    let vh = app.screen.visible_height();
    assert_eq!(vh, 4);
    let max_valid = lc - vh;

    app.screen.scroll_top = 99;

    let mut out: Vec<u8> = Vec::new();
    app.handle_resize(80, 6, &mut out).unwrap();
    assert!(
        app.screen.scroll_top <= max_valid,
        "after push+resize, scroll_top must clamp to <= max_valid={}; got {}",
        max_valid,
        app.screen.scroll_top
    );

    app.screen.scroll_top = 99;
    app.reveal_cursor();
    assert!(
        app.screen.scroll_top <= max_valid,
        "after push+reveal, scroll_top must clamp to <= max_valid={}; got {}",
        max_valid,
        app.screen.scroll_top
    );
}

#[test]
fn app_viewport_zero_size_regression_from_phase_2f_still_holds() {
    let mut app = App::new(None).unwrap();
    app.screen.scroll_top = 42;
    app.screen.scroll_left = 17;
    app.screen.width = 20;
    app.screen.height = 10;

    let mut out: Vec<u8> = Vec::new();
    app.handle_resize(0, 0, &mut out).unwrap();

    assert_eq!(app.screen.width, 0);
    assert_eq!(app.screen.height, 0);
    assert_eq!(
        app.screen.scroll_top, 0,
        "zero height still forces scroll_top=0"
    );
    assert_eq!(
        app.screen.scroll_left, 0,
        "zero width still forces scroll_left=0"
    );
    assert!(!out.is_empty());
}

#[test]
fn app_horiz_scroll_clamps_left_when_moving_to_shorter_line() {
    let mut app = App::new(None).unwrap();
    app.screen.width = 5;
    app.screen.height = 4;

    let mut sink: Vec<u8> = Vec::new();
    for c in "abc".chars() {
        app.handle_key_with(&mut sink, make_key(KeyCode::Char(c), KeyModifiers::NONE))
            .unwrap();
    }
    app.handle_key_with(&mut sink, make_key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    for c in "ABCDEFGHIJKLMNOPQRST".chars() {
        app.handle_key_with(&mut sink, make_key(KeyCode::Char(c), KeyModifiers::NONE))
            .unwrap();
    }
    assert_eq!(app.buffer.cursor().row, 1);

    for _ in 0..20 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Left, KeyModifiers::NONE))
            .unwrap();
    }
    for _ in 0..15 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Right, KeyModifiers::NONE))
            .unwrap();
    }
    let sl_long = app.screen.scroll_left;
    assert!(
        sl_long > 0,
        "need horiz scroll on long line; got {}",
        sl_long
    );

    app.handle_key_with(&mut sink, make_key(KeyCode::Up, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.buffer.cursor().row, 0);
    assert_eq!(
        app.screen.scroll_left, 0,
        "scroll_left must clamp to 0 after move to shorter line; got {}",
        app.screen.scroll_left
    );
}

#[test]
fn app_horiz_scroll_clamps_after_backspace_shortens_scrolled_line() {
    let mut app = App::new(None).unwrap();
    app.screen.width = 5;
    app.screen.height = 4;

    let mut sink: Vec<u8> = Vec::new();
    for c in "ABCDEFGHIJKLMNOPQRST".chars() {
        app.handle_key_with(&mut sink, make_key(KeyCode::Char(c), KeyModifiers::NONE))
            .unwrap();
    }
    for _ in 0..20 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Left, KeyModifiers::NONE))
            .unwrap();
    }
    for _ in 0..15 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Right, KeyModifiers::NONE))
            .unwrap();
    }
    assert!(app.screen.scroll_left > 0);

    for _ in 0..16 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Backspace, KeyModifiers::NONE))
            .unwrap();
    }

    let line_len = app.buffer.line(0).map(|s| s.chars().count()).unwrap_or(0);
    let vw = app.screen.visible_width();
    let max_allowed = line_len.saturating_add(1).saturating_sub(vw);
    assert!(
        app.screen.scroll_left <= max_allowed,
        "after bs shorten, scroll_left={} must <= {} (len={} vw={})",
        app.screen.scroll_left,
        max_allowed,
        line_len,
        vw
    );
    if line_len <= vw {
        assert_eq!(app.screen.scroll_left, 0);
    }
}
