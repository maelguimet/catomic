//! Purpose: map normalized terminal mouse events into cursor and selection actions.
//! Owns: viewport coordinate clamping, drag lifetime, and double-click word expansion.
//! Must not: enable terminal modes, mutate text, access clipboard, or inspect buffer internals.
//! Invariants: status-row clicks are ignored; mapped cursors stay within the active page.
//! Phase: 3-e mouse selection interaction.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::buffer::Cursor;
use crate::editor::selection::{word_bounds, Selection};
use crate::editor::text_layout;

const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(500);

pub(crate) fn handle_mouse(
    app: &mut super::super::App,
    out: &mut dyn Write,
    event: MouseEvent,
) -> io::Result<()> {
    match event.kind {
        MouseEventKind::ScrollUp => {
            return super::super::viewport::handle_mouse_wheel(
                app,
                out,
                super::super::viewport::ScrollDirection::Up,
                event.row as usize,
            );
        }
        MouseEventKind::ScrollDown => {
            return super::super::viewport::handle_mouse_wheel(
                app,
                out,
                super::super::viewport::ScrollDirection::Down,
                event.row as usize,
            );
        }
        _ => {}
    }
    if !super::super::view::source_is_displayed(app)
        || super::super::search::is_active(app)
        || super::super::command_prompt::is_active(app)
    {
        return Ok(());
    }
    if super::super::completion::cancel(app) {
        app.message = None;
    }
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => mouse_down(app, out, event),
        MouseEventKind::Drag(MouseButton::Left) => mouse_drag(app, out, event),
        MouseEventKind::Up(MouseButton::Left) => mouse_up(app, out, event),
        _ => Ok(()),
    }
}

fn mouse_down(
    app: &mut super::super::App,
    out: &mut dyn Write,
    event: MouseEvent,
) -> io::Result<()> {
    let Some(cursor) = map_mouse_cursor(app, event, false)? else {
        return Ok(());
    };
    let now = Instant::now();
    let is_double = app.selection.last_click.is_some_and(|(last, at)| {
        last == cursor && now.saturating_duration_since(at) <= DOUBLE_CLICK_WINDOW
    });
    if is_double {
        select_word(app, cursor);
        app.selection.last_click = None;
        app.selection.drag_anchor = None;
    } else {
        app.buffer.set_cursor(cursor);
        app.selection.range = None;
        app.selection.drag_anchor = Some(cursor);
        app.selection.last_click = Some((cursor, now));
    }
    app.reveal_cursor();
    app.render(out)
}

fn mouse_drag(
    app: &mut super::super::App,
    out: &mut dyn Write,
    event: MouseEvent,
) -> io::Result<()> {
    let Some(anchor) = app.selection.drag_anchor else {
        return Ok(());
    };
    let Some(cursor) = map_mouse_cursor(app, event, true)? else {
        return Ok(());
    };
    app.buffer.set_cursor(cursor);
    app.selection.range = Some(Selection::new(anchor, app.buffer.cursor()));
    app.reveal_cursor();
    app.render(out)
}

fn mouse_up(app: &mut super::super::App, out: &mut dyn Write, event: MouseEvent) -> io::Result<()> {
    let Some(anchor) = app.selection.drag_anchor.take() else {
        return Ok(());
    };
    let cursor = map_mouse_cursor(app, event, true)?.unwrap_or_else(|| app.buffer.cursor());
    app.buffer.set_cursor(cursor);
    let selection = Selection::new(anchor, app.buffer.cursor());
    app.selection.range = (!selection.is_empty()).then_some(selection);
    app.reveal_cursor();
    app.render(out)
}

fn select_word(app: &mut super::super::App, cursor: Cursor) {
    let line = app.buffer.line(cursor.row).unwrap_or_default();
    let (start_col, end_col) = word_bounds(&line, cursor.col);
    let start = Cursor {
        row: cursor.row,
        col: start_col,
    };
    let end = Cursor {
        row: cursor.row,
        col: end_col,
    };
    app.buffer.set_cursor(end);
    app.selection.range = Some(Selection::new(start, end));
}

fn map_mouse_cursor(
    app: &super::super::App,
    event: MouseEvent,
    clamp_status_row: bool,
) -> io::Result<Option<Cursor>> {
    let content_height = (app.screen.height as usize).saturating_sub(1);
    if content_height == 0 {
        return Ok(None);
    }
    let screen_row = event.row as usize;
    if screen_row >= content_height && !clamp_status_row {
        return Ok(None);
    }
    let visible_row = screen_row.min(content_height - 1);
    if super::super::view::soft_wrap_active(app) {
        return map_wrapped_cursor(app, visible_row, event.column as usize);
    }
    let row = app
        .screen
        .scroll_top
        .saturating_add(visible_row)
        .min(app.buffer.line_count().saturating_sub(1));
    let content_column =
        (event.column as usize).saturating_sub(super::super::view::gutter_width(app));
    let fetch_width = content_column.saturating_mul(4).saturating_add(32);
    let line = app
        .buffer
        .try_visible_lines_window(row, 1, app.screen.scroll_left, fetch_width)?
        .into_iter()
        .next()
        .map(|line| line.content)
        .unwrap_or_default();
    let relative_col = text_layout::scalar_at_cell(&line, content_column);
    let col = app
        .screen
        .scroll_left
        .saturating_add(relative_col)
        .min(app.buffer.line_char_count(row).unwrap_or(0));
    Ok(Some(Cursor { row, col }))
}

fn map_wrapped_cursor(
    app: &super::super::App,
    visible_row: usize,
    screen_column: usize,
) -> io::Result<Option<Cursor>> {
    let gutter = super::super::view::gutter_width(app);
    let width = super::super::view::content_width(app);
    let rows = crate::terminal::render::wrapped::visible_rows(
        super::super::view::display_buffer(app),
        app.screen.scroll_top,
        app.screen.wrap_col,
        app.screen.visible_height(),
        width,
    )?;
    let Some(row) = rows.get(visible_row) else {
        return Ok(None);
    };
    let cell = screen_column.saturating_sub(gutter);
    let relative = text_layout::scalar_at_cell(&row.content, cell);
    Ok(Some(Cursor {
        row: row.document_row,
        col: row.start_col.saturating_add(relative).min(row.end_col()),
    }))
}

#[cfg(test)]
mod tests;
