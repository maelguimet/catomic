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

const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(500);

pub(crate) fn handle_mouse(
    app: &mut super::super::App,
    out: &mut dyn Write,
    event: MouseEvent,
) -> io::Result<()> {
    if super::super::search::is_active(app) || super::super::command_prompt::is_active(app) {
        return Ok(());
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
    let Some(cursor) = map_mouse_cursor(app, event, false) else {
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
    let Some(cursor) = map_mouse_cursor(app, event, true) else {
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
    let cursor = map_mouse_cursor(app, event, true).unwrap_or_else(|| app.buffer.cursor());
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
) -> Option<Cursor> {
    let content_height = (app.screen.height as usize).saturating_sub(1);
    if content_height == 0 {
        return None;
    }
    let screen_row = event.row as usize;
    if screen_row >= content_height && !clamp_status_row {
        return None;
    }
    let visible_row = screen_row.min(content_height - 1);
    let row = app
        .screen
        .scroll_top
        .saturating_add(visible_row)
        .min(app.buffer.line_count().saturating_sub(1));
    let content_column =
        (event.column as usize).saturating_sub(super::super::view::gutter_width(app));
    let col = app
        .screen
        .scroll_left
        .saturating_add(content_column)
        .min(app.buffer.line_char_count(row).unwrap_or(0));
    Some(Cursor { row, col })
}

#[cfg(test)]
mod tests;
