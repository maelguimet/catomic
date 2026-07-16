//! Viewport / scroll / reveal helpers for App (Phase 2 slim).
//!
//! Purpose: owns the buffer-aware reveal + clamp + resize handling that
//! interacts with both Screen and the current Buffer cursor/line state.
//! Owns: handle_resize, reveal_cursor, clamp_viewport_to_buffer.
//! Must not: key dispatch, run loop, file state, render core.
//! Invariants: private to crate; called only from App impl; no behavior change.
//! Phase: 2 post-k slimming.

use std::io::Write;

use crate::app::App; // to mutate self.screen etc, or take pieces

/// Smallest helper seam for resize (and testability of it) without redesigning event loop.
/// Updates screen size, clamps for zero-size safety, reveals cursor, then renders.
pub(crate) fn handle_resize(
    app: &mut App,
    w: u16,
    h: u16,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    app.screen.update_size(w, h);
    app.screen.clamp_scroll();
    clamp_viewport_to_buffer(app);
    reveal_cursor(app);
    app.render(out)
}

/// Reveal the current cursor row/col so they are visible in the content area.
/// Called after cursor movement and content mutations (insert, delete, undo/redo).
/// Clamps first for zero-size terminals so reveal_* see a sane starting point.
pub(crate) fn reveal_cursor(app: &mut App) {
    app.screen.clamp_scroll();
    clamp_viewport_to_buffer(app);
    let c = super::view::display_buffer(app).cursor();
    app.screen.reveal_row(c.row);
    app.screen
        .reveal_col_with_width(c.col, super::view::content_width(app));
    // Re-clamp after reveal: reveal_col may target a col on a now-shorter line,
    // leaving scroll_left > (line_len - vw). Clamp pulls it back.
    clamp_viewport_to_buffer(app);
}

/// Buffer-aware clamp so scroll offsets cannot exceed useful buffer content.
/// Vertical: if vh==0 => 0; if line_count <= vh => 0; else scroll_top <= line_count - vh.
/// Horizontal (scalar chars): clamp scroll_left using current cursor line char count.
/// Uses line_len + 1 - vw (saturating) to match reveal_col end-of-line math and
/// keep cursor revealed; clamps excess from prior long lines after move/delete/shrink.
/// Private helper keeps Screen buffer-agnostic.
/// Called from resize and reveal paths.
pub(crate) fn clamp_viewport_to_buffer(app: &mut App) {
    // Vertical
    let (lc, line_len) = {
        let display = super::view::display_buffer(app);
        let cursor = display.cursor();
        (
            display.line_count(),
            display.line_char_count(cursor.row).unwrap_or(0),
        )
    };
    let vh = app.screen.visible_height();
    if vh == 0 || lc <= vh {
        app.screen.scroll_top = 0;
    } else {
        let max_top = lc - vh;
        if app.screen.scroll_top > max_top {
            app.screen.scroll_top = max_top;
        }
    }

    // Horizontal: scalar char count on the *current cursor line* only.
    // Matches Phase 2 scalar limitation; movement/delete/undo to shorter line
    // must not leave scroll_left stranded.
    // Max uses line_len + 1 - vw (saturating) to be consistent with reveal_col's
    // "col + 1 - vw" for end-of-line cursor (col can == line_len). This preserves
    // reveal behavior and keeps cursor visible while still clamping high scrolls
    // on shorter lines to avoid empty space.
    let vw = super::view::content_width(app);
    if vw == 0 {
        app.screen.scroll_left = 0;
    } else {
        let max_left = line_len.saturating_add(1).saturating_sub(vw);
        if app.screen.scroll_left > max_left {
            app.screen.scroll_left = max_left;
        }
    }
}
