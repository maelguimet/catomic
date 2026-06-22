//! App tests (child module split out of app.rs).
//!
//! Purpose: this file must contain the tests for App high-level state, key handling,
//! resize/reveal/scroll invariants, dirty tracking, quit guard, and render seams.
//! Owns: all cfg(test) tests and the make_key helper for simulated input.
//! Must not: contain any runtime logic or be included outside test builds.
//! Invariants: loaded only under #[cfg(test)] via `mod tests;` in app.rs;
//!              uses `use super::*;` to access private App methods (e.g. handle_key_with).
//! Phase: 2-g cleanup (no behavior change).

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[test]
fn app_file_state_new_starts_clean() {
    let app = App::new(None).unwrap();
    assert!(!app.file.dirty, "new app without path starts clean");
    assert!(app.file.path.is_none());
    // screen field added in 2-c; verify default here too (no behavior change)
    assert_eq!(app.screen.height, 24);
    assert_eq!(app.screen.scroll_top, 0);

    let app2 = App::new(Some("existing.txt")).unwrap();
    assert!(!app2.file.dirty, "open (even missing file) starts clean");
    assert_eq!(
        app2.file.path.as_deref(),
        Some(std::path::Path::new("existing.txt"))
    );
}

#[test]
fn app_dirty_lifecycle_via_keys() {
    // Use explicit temp path for the test so we NEVER write bare "untitled.txt"
    // into the repo cwd. App::new with a path (even non-existing) starts clean
    // and save will target that path instead of defaulting.
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_test_dirty_lifecycle_{}_{}.txt",
        std::process::id(),
        "lifecycle"
    ));
    let test_path = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&test_path); // ensure clean start

    let mut app = App::new(Some(&test_path)).unwrap();
    assert!(!app.file.dirty);
    assert_eq!(
        app.file.path.as_deref(),
        Some(std::path::Path::new(&test_path))
    );

    // char insert marks dirty
    app.handle_key(KeyEvent {
        code: KeyCode::Char('a'),
        modifiers: KeyModifiers::NONE,
        kind: crossterm::event::KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    })
    .unwrap();
    assert!(app.file.dirty, "edit marks dirty");

    // save (via atomic) clears dirty; uses explicit path (no untitled.txt)
    app.handle_key(KeyEvent {
        code: KeyCode::Char('s'),
        modifiers: KeyModifiers::CONTROL,
        kind: crossterm::event::KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    })
    .unwrap();
    assert!(!app.file.dirty, "successful save marks clean");
    assert!(app.file.path.is_some());

    // edit after save marks dirty again
    app.handle_key(KeyEvent {
        code: KeyCode::Char('b'),
        modifiers: KeyModifiers::NONE,
        kind: crossterm::event::KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    })
    .unwrap();
    assert!(app.file.dirty, "post-save edit marks dirty again");

    // Clean up ONLY the temp path created/used by this test.
    let _ = std::fs::remove_file(&test_path);
}

// Phase 2-b quit guard + message tests (via simulated keys; no real terminal)

fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: crossterm::event::KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    }
}

#[test]
fn app_quit_clean_immediately() {
    let mut app = App::new(None).unwrap();
    assert!(!app.file.dirty);
    assert!(!app.should_quit);
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.should_quit, "clean Ctrl+Q quits immediately");
}

#[test]
fn app_quit_dirty_first_sets_pending_and_message_second_quits() {
    let mut app = App::new(None).unwrap();
    // make dirty
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    assert!(!app.pending_quit_confirm);
    assert!(app.message.is_none());

    // first Ctrl+Q: no quit, sets pending + msg
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.should_quit, "first dirty Q does not quit");
    assert!(app.pending_quit_confirm);
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("Unsaved changes") && msg.contains("Ctrl+Q again"),
        "message should warn: got {:?}",
        app.message
    );

    // second Ctrl+Q: quits
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.should_quit, "second dirty Q quits");
}

#[test]
fn app_dirty_ctrl_q_first_renders_warning_immediately() {
    // Regression for invisible warning: first dirty Ctrl+Q must emit render
    // containing the message on bottom row (via the writer seam).
    let mut app = App::new(None).unwrap();
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    assert!(app.message.is_none());

    let mut out: Vec<u8> = Vec::new();
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('q'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert!(!app.should_quit, "first dirty Q does not quit");
    assert!(app.pending_quit_confirm);
    let rendered = String::from_utf8_lossy(&out);
    assert!(
        rendered.contains("Unsaved changes") && rendered.contains("Ctrl+Q again"),
        "warning message text must appear in render output"
    );
    assert!(
        rendered.contains("\x1b[K"),
        "render must clear bottom row with \\x1b[K even for message"
    );
}

#[test]
fn app_ctrl_s_after_dirty_clears_dirty_and_pending() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_test_save_clears_pending_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);

    // trigger quit warn
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_quit_confirm);

    // Ctrl+S: success clears dirty + pending + msg
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);
    assert!(!app.pending_quit_confirm);
    assert!(app.message.is_none());

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_save_error_keeps_dirty_and_sets_error_message() {
    // Use a dedicated subdir under temp (never bare temp_dir or root sibling)
    // so that path points to a directory -> atomic_write fails as intended.
    let mut bad = std::env::temp_dir();
    bad.push(format!("catomic_bad_save_dir_{}", std::process::id()));
    // ensure clean and is a dir
    let _ = std::fs::remove_dir_all(&bad);
    std::fs::create_dir_all(&bad).expect("create dedicated bad dir");
    assert!(bad.is_dir());

    let mut app = App::new(None).unwrap();
    app.file.path = Some(bad.clone());
    app.file.dirty = true;
    app.message = None;

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(app.file.dirty, "save error must keep dirty=true");
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("Save error") || msg.contains("error"),
        "save error should set message, got: {:?}",
        app.message
    );

    // cleanup dedicated dir only
    let _ = std::fs::remove_dir_all(&bad);
}

#[test]
fn app_edit_after_quit_warning_clears_pending() {
    let mut app = App::new(None).unwrap();
    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);

    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_quit_confirm);
    assert!(app.message.is_some());

    // content-mutating edit clears BOTH pending and message (movements do not)
    app.handle_key(make_key(KeyCode::Char('!'), KeyModifiers::NONE))
        .unwrap();
    assert!(
        !app.pending_quit_confirm,
        "edit after warning clears pending"
    );
    assert!(
        app.message.is_none(),
        "edit after warning also clears stale message"
    );
}

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
