//! App viewport/scroll/resize/reveal tests (child submodule of app::tests).
//!
//! Purpose: this file must contain the App-level tests for viewport behavior
//! (scroll_top/scroll_left, reveal_cursor, resize, zero-size handling, horiz/vert clamp).
//! Owns: all viewport/scroll/resize/reveal/clamp related #[test] functions.
//! Must not: contain runtime logic or be included outside test builds.
//! Invariants: loaded as mod viewport; from tests.rs under #[cfg(test)] mod tests;
//!              uses `use super::super::*;` to reach private App methods and `use super::make_key;`
//!              for the shared key helper (kept pub(super) in hub).
//! Phase: 2-i narrow cleanup (no behavior or API change; test names preserved).

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
    // set non-default height (no real term)
    app.screen.height = 10;
    app.screen.scroll_top = 0;

    // trigger render via content path that calls render (uses handle_key_with seam)
    let mut out: Vec<u8> = Vec::new();
    app.handle_key_with(&mut out, make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    let rendered = String::from_utf8_lossy(&out);
    // bottom row clear/pos for height=10
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

// Phase 2-d app-level reveal/scroll_top tests (via seams + captured render)

#[test]
fn app_cursor_down_past_visible_updates_scroll_top() {
    let mut app = App::new(None).unwrap();
    // Small content viewport: height=6 => visible_height=5 content rows (0..4)
    app.screen.height = 6;
    app.screen.scroll_top = 0;

    // Create 10 lines (0..9) by newlines; cursor ends after last insert at end of last line.
    // Use Enter key via seam to exercise the path that does reveal (captures output, keeps test quiet).
    let mut sink: Vec<u8> = Vec::new();
    for _ in 0..9 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
    }
    // Now we have 10 lines (rows 0-9), cursor at row=9, col=0 (after 9 newlines from empty start)
    assert_eq!(app.buffer.cursor().row, 9);

    // With vh=5, row 9 is way below (0+5=5), so reveal must have scrolled on last Enter.
    // scroll_top should be at least 9 +1 -5 = 5
    assert!(
        app.screen.scroll_top >= 5,
        "down past viewport must update scroll_top; got {}",
        app.screen.scroll_top
    );
}

#[test]
fn app_render_after_reveal_omits_earlier_lines_and_shows_cursor_row() {
    let mut app = App::new(None).unwrap();
    app.screen.height = 6; // vh=5
    app.screen.scroll_top = 0;

    // Build lines with unique markers: insert "L0\nL1\n...L9"
    // Simpler: repeated Enter then type a marker char on each line? Use direct buffer for setup clarity.
    // Then drive a Down that will reveal via the key path.
    for i in 0..10 {
        if i > 0 {
            app.buffer.insert_newline();
        }
        // put a distinguishable token at start of each line
        app.buffer.insert_char('L');
        // i as rough marker by repeating a char; keep simple: use digits for later lines
        let marker = char::from(b'0' + (i % 10) as u8);
        app.buffer.insert_char(marker);
    }
    // cursor now at row=9, col=2 on "L9"
    assert_eq!(app.buffer.cursor().row, 9);

    // Force a scroll by simulating many downs via keys (each calls reveal_cursor)
    // Use handle_key_with + sink to exercise reveal path without spamming test stdout.
    let mut sink: Vec<u8> = Vec::new();
    // Start from top by resetting scroll; then down past.
    app.screen.scroll_top = 0;
    // Move up to row 0 first (we are at 9), then down 9 times with small vh to trigger reveal on the way.
    for _ in 0..9 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Up, KeyModifiers::NONE))
            .unwrap();
    }
    assert_eq!(app.buffer.cursor().row, 0);
    app.screen.scroll_top = 0;

    // Now move down past the visible area
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

    // Capture a render; earlier lines (e.g. L0) must not be in the emitted content region.
    let mut out: Vec<u8> = Vec::new();
    app.render(&mut out).unwrap();
    let rendered = String::from_utf8_lossy(&out);

    // The render writes visible_lines(scroll_top, content_h). First line content after clear should not be L0/L1 if scrolled.
    // Check absence of a unique early marker that would be before scroll_top.
    assert!(
        !rendered.contains("L0"),
        "early line content must not be emitted when scrolled; scroll_top={}\nout: {}",
        app.screen.scroll_top,
        rendered
    );
    // Cursor row's content should be present (L9 or similar)
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
    // Create 16 lines (0..15) with cursor at row 15
    for _ in 0..15 {
        app.buffer.insert_newline();
    }
    assert_eq!(app.buffer.cursor().row, 15);
    // Large viewport so currently no scroll
    app.screen.height = 30;
    app.screen.scroll_top = 0;

    // Now resize to a small height where 15 would be offscreen if not revealed.
    // height=10 => vh=9; 15 >= 0+9 => reveal will set scroll_top = 15+1-9=7
    let mut out: Vec<u8> = Vec::new();
    app.handle_resize(40, 10, &mut out).unwrap();

    assert_eq!(app.screen.height, 10);
    assert!(
        app.screen.scroll_top > 0,
        "resize to smaller must reveal; scroll_top={}",
        app.screen.scroll_top
    );
    // 15 should now be inside [scroll_top, scroll_top+8]
    let vh = app.screen.visible_height();
    assert!(
        app.screen.scroll_top <= 15 && 15 < app.screen.scroll_top + vh,
        "cursor row 15 must be visible after small resize; scroll_top={}, vh={}",
        app.screen.scroll_top,
        vh
    );
    assert!(!out.is_empty(), "resize must render");
}

// Phase 2-e app/render horizontal reveal tests

#[test]
fn app_typing_past_visible_width_updates_scroll_left() {
    let mut app = App::new(None).unwrap();
    app.screen.width = 5; // vw=5
    app.screen.height = 6;
    app.screen.scroll_left = 0;

    // Type 8 chars on one line: cursor col will become 8
    let mut sink: Vec<u8> = Vec::new();
    for _ in 0..8 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Char('x'), KeyModifiers::NONE))
            .unwrap();
    }
    assert_eq!(app.buffer.cursor().col, 8);
    // vw=5; col=8 >= 0+5 => scroll_left should become 8+1-5=4
    assert!(
        app.screen.scroll_left >= 4,
        "typing past width must update scroll_left; got {}",
        app.screen.scroll_left
    );
}

#[test]
fn app_render_after_horizontal_reveal_omits_earlier_chars_and_shows_cursor_side() {
    let mut app = App::new(None).unwrap();
    app.screen.width = 6; // vw=6
    app.screen.height = 4;
    app.screen.scroll_left = 0;

    // Build a long distinguishable line: "0123456789ABCDEF" (16 chars), cursor at end col=16
    for c in "0123456789ABCDEF".chars() {
        app.buffer.insert_char(c);
    }
    assert_eq!(app.buffer.cursor().col, 16);

    // Force reveal via down/up or just call (but use key path for full)
    // Move left then right many to trigger reveals; simpler: direct reveal then capture render.
    // But per pattern, drive with keys to exercise reveal_cursor.
    let mut sink: Vec<u8> = Vec::new();
    // Ensure at far right
    // We are already at col=16. A Right does nothing (clamp?), Left then Rights to re-reveal.
    for _ in 0..5 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Left, KeyModifiers::NONE))
            .unwrap();
    }
    // now col ~11; scroll may be 0 still. Move right past edge.
    for _ in 0..10 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Right, KeyModifiers::NONE))
            .unwrap();
    }
    assert!(
        app.screen.scroll_left > 0,
        "must have scrolled horizontally; scroll_left={}",
        app.screen.scroll_left
    );

    // Capture render output
    let mut out: Vec<u8> = Vec::new();
    app.render(&mut out).unwrap();
    let rendered = String::from_utf8_lossy(&out);

    // Early chars (e.g. "01") should be omitted from content region when scrolled
    // (they are before scroll_left)
    assert!(
        !rendered.contains("01"),
        "early line chars must be omitted after horiz scroll; scroll_left={}\n{}",
        app.screen.scroll_left,
        rendered
    );
    // Cursor side content should be present (near end, e.g. some later digit/letter)
    // At least one char from the right side should appear.
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
    // Long line, cursor at high col
    for c in "ABCDEFGHIJKLMNOP".chars() {
        app.buffer.insert_char(c);
    }
    assert_eq!(app.buffer.cursor().col, 16);
    // Wide viewport: no scroll
    app.screen.width = 30;
    app.screen.scroll_left = 0;

    // Resize narrower: width=5 => vw=5; 16 >=0+5 => reveal sets scroll_left=16+1-5=12
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

// Phase 2-f: zero-size resize + clamp + post-resize normal reveal, and horiz scroll shrink on delete/bs

#[test]
fn app_resize_to_zero_size_clamps_scroll_and_does_not_panic() {
    let mut app = App::new(None).unwrap();
    // Set some nonzero scroll
    app.screen.scroll_top = 5;
    app.screen.scroll_left = 12;
    app.screen.width = 20;
    app.screen.height = 10;

    // Resize to 0x0 via seam (no real term)
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
    // render on zero size must be safe (no panic, some output for clear/pos)
    assert!(!out.is_empty());
}

#[test]
fn app_resize_to_zero_then_back_to_nonzero_typing_and_move_reveal_normally() {
    let mut app = App::new(None).unwrap();
    app.screen.width = 8;
    app.screen.height = 6;
    app.screen.scroll_left = 0;
    app.screen.scroll_top = 0;

    // Go to zero
    let mut sink: Vec<u8> = Vec::new();
    app.handle_resize(0, 0, &mut sink).unwrap();
    assert_eq!(app.screen.scroll_top, 0);
    assert_eq!(app.screen.scroll_left, 0);

    // Back to usable size
    app.handle_resize(10, 8, &mut sink).unwrap(); // vh=7, vw=10
    assert_eq!(app.screen.width, 10);
    assert_eq!(app.screen.height, 8);

    // Type enough to scroll horizontally, then move and type more; reveal must keep working
    for _ in 0..12 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Char('x'), KeyModifiers::NONE))
            .unwrap();
    }
    assert!(
        app.screen.scroll_left > 0,
        "should have horiz scrolled while typing"
    );
    // Move left a few; reveal should reduce scroll_left if cursor goes before viewport
    for _ in 0..8 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Left, KeyModifiers::NONE))
            .unwrap();
    }
    // After moving left of the old viewport start, scroll_left should have decreased
    // (exact 0 not required; it must not be stuck high)
    assert!(
        app.screen.scroll_left < 5,
        "moving left after horiz scroll should reduce scroll_left; got {}",
        app.screen.scroll_left
    );

    // Down/up and insert should still reveal without panic on nonzero size
    app.handle_key_with(&mut sink, make_key(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(&mut sink, make_key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(app.screen.scroll_top <= app.buffer.cursor().row);
}

#[test]
fn app_delete_and_backspace_after_horiz_scroll_reduce_scroll_left_when_cursor_before_viewport() {
    let mut app = App::new(None).unwrap();
    app.screen.width = 5; // vw=5
    app.screen.height = 4;
    app.screen.scroll_left = 0;

    // Build a line longer than width and scroll to have content on right
    let mut sink: Vec<u8> = Vec::new();
    for c in "ABCDEFGHIJKLMNOPQRST".chars() {
        // 20 chars, col ends at 20
        app.buffer.insert_char(c);
    }
    // cursor col=20; force reveal via keys to set scroll
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

    // Now delete_back (backspace) which moves cursor left. If cursor moves before current viewport,
    // reveal must reduce scroll_left.
    // Do enough backspaces to cross before the viewport start.
    // Current cursor after the rights: we did 20 left then 15 right => col = 15 (started at 20 after inserts)
    // Simpler: backspace repeatedly and check scroll decreases when appropriate.
    let mut last_sl = app.screen.scroll_left;
    for _ in 0..10 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Backspace, KeyModifiers::NONE))
            .unwrap();
        if app.buffer.cursor().col < last_sl {
            // once cursor is before the old scroll window, reveal should have pulled scroll_left down
            assert!(
                app.screen.scroll_left <= last_sl,
                "backspace moving cursor before viewport should not increase scroll_left"
            );
        }
        last_sl = app.screen.scroll_left;
    }

    // Also exercise delete forward from a scrolled position (moves content left of cursor)
    // Reset a scrolled state: move to a high col again
    app.screen.scroll_left = 8;
    app.buffer.move_right(); // may clamp internally but ok
    let pre = app.screen.scroll_left;
    app.handle_key_with(&mut sink, make_key(KeyCode::Delete, KeyModifiers::NONE))
        .unwrap();
    // Delete forward does not move cursor col, but may change content; scroll_left should stay sensible (no increase here)
    assert!(app.screen.scroll_left <= pre + 1); // allow small tolerance; main is no explosion
}

// Phase 2-h: buffer-aware viewport clamp tests (vertical first)

#[test]
fn app_viewport_clamp_scroll_top_to_zero_when_buffer_shorter_than_viewport() {
    let mut app = App::new(None).unwrap();
    // 3 lines total (start + 2 newlines)
    app.buffer.insert_newline();
    app.buffer.insert_newline();
    assert_eq!(app.buffer.line_count(), 3);

    app.screen.height = 11; // vh = 10
    app.screen.scroll_top = 5; // push beyond valid (max should be 0)

    app.reveal_cursor();

    assert_eq!(
        app.screen.scroll_top, 0,
        "buffer shorter than viewport must clamp scroll_top to 0"
    );
}

#[test]
fn app_viewport_clamps_scroll_top_after_manual_push_then_resize_or_reveal() {
    let mut app = App::new(None).unwrap();
    // Build 20 lines (0..19), cursor ends at row 19
    for _ in 0..19 {
        app.buffer.insert_newline();
    }
    assert_eq!(app.buffer.cursor().row, 19);
    let lc = app.buffer.line_count();
    assert_eq!(lc, 20);

    app.screen.height = 6; // vh=5
    let vh = app.screen.visible_height();
    assert_eq!(vh, 5);
    let max_valid = lc - vh; // 15

    // Manually push beyond
    app.screen.scroll_top = 99;

    // Via resize seam
    let mut out: Vec<u8> = Vec::new();
    app.handle_resize(80, 6, &mut out).unwrap();
    assert!(
        app.screen.scroll_top <= max_valid,
        "after push+resize, scroll_top must clamp to <= max_valid={}; got {}",
        max_valid,
        app.screen.scroll_top
    );

    // Via direct reveal after re-push
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
    // Explicit regression: the 2-f zero cases must continue to hold after buffer clamp.
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

// Phase 2-h horizontal clamp cases (shorter line on move/delete)

#[test]
fn app_horiz_scroll_clamps_left_when_moving_to_shorter_line() {
    let mut app = App::new(None).unwrap();
    app.screen.width = 5; // vw=5
    app.screen.height = 4;

    // Build: line 0 short "abc" (len=3 <=vw), line 1 long
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
    // cursor row=1, col=20
    assert_eq!(app.buffer.cursor().row, 1);

    // Establish horiz scroll on long line (lefts then rights)
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

    // Move up to shorter line; reveal must clamp scroll_left (len=3 <=5 => 0)
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
    app.screen.width = 5; // vw=5
    app.screen.height = 4;

    let mut sink: Vec<u8> = Vec::new();
    // Long line
    for c in "ABCDEFGHIJKLMNOPQRST".chars() {
        app.handle_key_with(&mut sink, make_key(KeyCode::Char(c), KeyModifiers::NONE))
            .unwrap();
    }
    // scroll to right
    for _ in 0..20 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Left, KeyModifiers::NONE))
            .unwrap();
    }
    for _ in 0..15 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Right, KeyModifiers::NONE))
            .unwrap();
    }
    assert!(app.screen.scroll_left > 0);

    // Backspace enough to make line short (20 -> ~4)
    for _ in 0..16 {
        app.handle_key_with(&mut sink, make_key(KeyCode::Backspace, KeyModifiers::NONE))
            .unwrap();
    }

    let line_len = app.buffer.line(0).map(|s| s.chars().count()).unwrap_or(0);
    let vw = app.screen.visible_width();
    // With our clamp using +1-vw, max is line_len+1-vw or 0
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
