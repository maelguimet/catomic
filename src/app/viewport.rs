//! Viewport / scroll / reveal helpers for App (Phase 2 slim).
//!
//! Purpose: owns the buffer-aware reveal + clamp + resize handling that
//! interacts with both Screen and the current Buffer cursor/line state.
//! Owns: resize, cursor reveal, viewport-only wheel scrolling, and bounds clamping.
//! Must not: key dispatch, run loop, file state, render core.
//! Invariants: viewport-only scrolling never mutates a display buffer cursor or source state.
//! Phase: post-v0.1 viewport-only wheel scrolling.

use std::io::Write;

use crate::app::App; // to mutate self.screen etc, or take pieces

/// Crossterm normalizes terminal and tmux wheel reports to one event per wheel step.
/// Keep the visible-row amount centralized so configuration can replace it later.
pub(crate) const MOUSE_WHEEL_ROWS: usize = 3;

#[derive(Clone, Copy)]
pub(crate) enum ScrollDirection {
    Up,
    Down,
}

/// Smallest helper seam for resize (and testability of it) without redesigning event loop.
/// Updates screen size, clamps for zero-size safety, preserves an off-screen cursor,
/// and retains normal cursor-follow behavior when the cursor was visible.
pub(crate) fn handle_resize(
    app: &mut App,
    w: u16,
    h: u16,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    let cursor_was_visible = display_cursor_is_visible(app);
    app.screen.update_size(w, h);
    super::view::relayout_preview(app);
    app.screen.clamp_scroll();
    clamp_viewport_to_buffer(app);
    if cursor_was_visible {
        reveal_cursor(app);
    }
    app.render(out)
}

/// Re-query size and redraw after a terminal foreground/focus transition.
/// No prompt, edit, task, or unsaved state is recreated.
pub(crate) fn redraw_after_focus(
    app: &mut App,
    size: Option<(u16, u16)>,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    if let Some((width, height)) = size {
        app.screen.update_size(width, height);
        super::view::relayout_preview(app);
        app.screen.clamp_scroll();
        clamp_viewport_to_buffer(app);
    }
    app.render(out)
}

pub(crate) fn handle_mouse_wheel(
    app: &mut App,
    out: &mut dyn Write,
    direction: ScrollDirection,
    terminal_row: usize,
) -> std::io::Result<()> {
    if terminal_row >= app.screen.visible_height() || mouse_wheel_is_blocked(app) {
        return Ok(());
    }
    if scroll_viewport(app, direction, MOUSE_WHEEL_ROWS)? {
        app.render(out)?;
    }
    Ok(())
}

pub(crate) fn scroll_viewport(
    app: &mut App,
    direction: ScrollDirection,
    rows: usize,
) -> std::io::Result<bool> {
    if rows == 0 || app.screen.visible_height() == 0 {
        return Ok(false);
    }
    if super::view::soft_wrap_active(app) {
        scroll_wrapped_viewport(app, direction, rows)
    } else {
        Ok(scroll_logical_viewport(app, direction, rows))
    }
}

/// Scroll by a signed number of visual rows and render once.
/// Used by touch actions whose direction is encoded in the sign.
pub(crate) fn scroll_view(app: &mut App, out: &mut dyn Write, rows: isize) -> std::io::Result<()> {
    let direction = if rows < 0 {
        ScrollDirection::Up
    } else {
        ScrollDirection::Down
    };
    if scroll_viewport(app, direction, rows.unsigned_abs())? {
        app.render(out)?;
    }
    Ok(())
}

fn scroll_logical_viewport(app: &mut App, direction: ScrollDirection, rows: usize) -> bool {
    let maximum = super::view::display_buffer(app)
        .line_count()
        .saturating_sub(app.screen.visible_height());
    let before = app.screen.scroll_top;
    app.screen.scroll_top = match direction {
        ScrollDirection::Up => before.saturating_sub(rows),
        ScrollDirection::Down => before.saturating_add(rows).min(maximum),
    };
    app.screen.scroll_top != before
}

fn scroll_wrapped_viewport(
    app: &mut App,
    direction: ScrollDirection,
    rows: usize,
) -> std::io::Result<bool> {
    let before = (app.screen.scroll_top, app.screen.wrap_col);
    let forward = matches!(direction, ScrollDirection::Down);
    let origin = crate::terminal::render::wrapped::scroll_origin(
        super::view::display_buffer(app),
        before.0,
        before.1,
        app.screen.visible_height(),
        super::view::content_width(app),
        rows,
        forward,
    )?;
    app.screen.scroll_top = origin.0;
    app.screen.wrap_col = origin.1;
    Ok(origin != before)
}

fn mouse_wheel_is_blocked(app: &App) -> bool {
    super::search::is_active(app)
        || super::command_prompt::is_active(app)
        || super::replace::is_active(app)
        || super::completion::is_active(app)
        || app.pending_llm_request.is_some()
        || app.llm_task.is_some()
        || app.repo_llm_state.is_some()
        || super::external_command::is_running(app)
}

fn display_cursor_is_visible(app: &App) -> bool {
    let buffer = super::view::display_buffer(app);
    if super::view::soft_wrap_active(app) {
        return crate::terminal::render::wrapped::cursor_is_visible(
            buffer,
            app.screen.scroll_top,
            app.screen.wrap_col,
            app.screen.visible_height(),
            super::view::content_width(app),
        )
        .unwrap_or(false);
    }
    let cursor = buffer.cursor();
    cursor.row >= app.screen.scroll_top
        && cursor.row
            < app
                .screen
                .scroll_top
                .saturating_add(app.screen.visible_height())
}

/// Reveal the current cursor row/col so they are visible in the content area.
/// Called after cursor movement and content mutations (insert, delete, undo/redo).
/// Clamps first for zero-size terminals so reveal_* see a sane starting point.
pub(crate) fn reveal_cursor(app: &mut App) {
    app.screen.clamp_scroll();
    clamp_viewport_to_buffer(app);
    let c = super::view::display_buffer(app).cursor();
    app.screen.reveal_row(c.row);
    if super::view::soft_wrap_active(app) {
        reveal_wrapped_cursor(app);
        return;
    }
    app.screen.wrap_col = 0;
    app.screen
        .reveal_col_with_width(c.col, super::view::content_width(app));
    // Re-clamp after reveal: reveal_col may target a col on a now-shorter line,
    // leaving scroll_left > (line_len - vw). Clamp pulls it back.
    clamp_viewport_to_buffer(app);
    reveal_horizontal_cells(app);
}

#[cfg(test)]
mod tests;

fn reveal_wrapped_cursor(app: &mut App) {
    app.screen.scroll_left = 0;
    let height = app.screen.visible_height();
    let width = super::view::content_width(app);
    let buffer = super::view::display_buffer(app);
    let visible = crate::terminal::render::wrapped::cursor_is_visible(
        buffer,
        app.screen.scroll_top,
        app.screen.wrap_col,
        height,
        width,
    )
    .unwrap_or(false);
    if visible {
        return;
    }
    let cursor = buffer.cursor();
    let wrap_col =
        crate::terminal::render::wrapped::start_col_near_cursor(buffer, cursor, height, width)
            .unwrap_or(cursor.col);
    app.screen.scroll_top = cursor.row;
    app.screen.wrap_col = wrap_col;
}

fn reveal_horizontal_cells(app: &mut App) {
    let width = super::view::content_width(app);
    if width == 0 {
        app.screen.scroll_left = 0;
        return;
    }
    let cursor = super::view::display_buffer(app).cursor();
    let context_start = app.screen.scroll_left.saturating_sub(64);
    let relative_cursor = cursor.col.saturating_sub(context_start);
    let fetch_width = relative_cursor.saturating_add(65);
    let Ok(lines) = super::view::display_buffer(app).try_visible_lines_window(
        cursor.row,
        1,
        context_start,
        fetch_width,
    ) else {
        return;
    };
    let Some(line) = lines.first() else {
        return;
    };
    let requested_start = app.screen.scroll_left.saturating_sub(context_start);
    let boundary_start =
        crate::editor::text_layout::ceil_to_grapheme_col(&line.content, requested_start);
    let visible: String = line.content.chars().skip(boundary_start).collect();
    let cursor_in_visible = cursor
        .col
        .saturating_sub(context_start.saturating_add(boundary_start));
    let cursor_cells = crate::editor::text_layout::scalar_to_cell(&visible, cursor_in_visible);
    let hidden_cells = cursor_cells.saturating_add(1).saturating_sub(width);
    let advance = crate::editor::text_layout::scalar_at_cell(&visible, hidden_cells);
    app.screen.scroll_left = context_start
        .saturating_add(boundary_start)
        .saturating_add(advance);
}

/// Buffer-aware clamp so scroll offsets cannot exceed useful buffer content.
/// Vertical: logical views keep a full final viewport; wrapped views retain their
/// logical-row/wrap-column origin because logical height does not bound visual rows.
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
    if super::view::soft_wrap_active(app) {
        app.screen.scroll_top = app.screen.scroll_top.min(lc.saturating_sub(1));
        let start_len = super::view::display_buffer(app)
            .line_char_count(app.screen.scroll_top)
            .unwrap_or(0);
        app.screen.wrap_col = app.screen.wrap_col.min(start_len);
    } else if vh == 0 || lc <= vh {
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
