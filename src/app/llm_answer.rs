//! Purpose: this file must present model explanations as transient read-only text.
//! Owns: answer view state, navigation, paste guards, and source viewport restoration.
//! Must not: mutate source/history, create clients, call endpoints, or apply output.
//! Invariants: no key applies an answer; Escape closes and restores the source viewport.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Buffer, Cursor, PieceTable};

pub(crate) struct AnswerView {
    buffer: PieceTable,
    source_scroll_top: usize,
    source_scroll_left: usize,
}

pub(crate) fn show(app: &mut super::App, out: &mut dyn Write, answer: &str) -> io::Result<()> {
    if answer.trim().is_empty() {
        app.message_info("The model returned an empty explanation.");
        return app.render(out);
    }
    super::view::cancel_preview(app);
    super::lint::close_view(app);
    super::project_files::close_view(app);
    super::llm_preview::close(app);
    close(app);
    app.surfaces.llm_answer = Some(AnswerView {
        buffer: PieceTable::from_text(answer),
        source_scroll_top: app.screen.scroll_top,
        source_scroll_left: app.screen.scroll_left,
    });
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    app.selection.clear();
    app.message_info("LLM explanation (read-only). Esc closes.");
    app.render(out)
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if !is_viewing(app) || is_quit(key) {
        return Ok(false);
    }
    if key.code == KeyCode::Esc {
        close(app);
        app.message = None;
        app.reveal_cursor();
        app.render(out)?;
        return Ok(true);
    }
    match key.code {
        KeyCode::Left => move_cursor(app, Move::Left),
        KeyCode::Right => move_cursor(app, Move::Right),
        KeyCode::Up => move_cursor(app, Move::Up),
        KeyCode::Down => move_cursor(app, Move::Down),
        KeyCode::PageUp => move_page(app, false),
        KeyCode::PageDown => move_page(app, true),
        KeyCode::Home => set_line_edge(app, false),
        KeyCode::End => set_line_edge(app, true),
        _ => app.message_info("LLM explanation is read-only; Esc closes."),
    }
    reveal_cursor(app);
    app.render(out)?;
    Ok(true)
}

pub(crate) fn handle_paste(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    if !is_viewing(app) {
        return Ok(false);
    }
    app.message_info("LLM explanation is read-only; Esc closes.");
    app.render(out)?;
    Ok(true)
}

pub(crate) fn is_viewing(app: &super::App) -> bool {
    app.surfaces.llm_answer.is_some()
}

pub(crate) fn display_buffer(app: &super::App) -> Option<&dyn Buffer> {
    app.surfaces
        .llm_answer
        .as_ref()
        .map(|answer| &answer.buffer as &dyn Buffer)
}

pub(crate) fn close(app: &mut super::App) -> bool {
    if let Some(answer) = app.surfaces.llm_answer.take() {
        app.screen.scroll_top = answer.source_scroll_top;
        app.screen.scroll_left = answer.source_scroll_left;
        true
    } else {
        false
    }
}

enum Move {
    Left,
    Right,
    Up,
    Down,
}

fn move_cursor(app: &mut super::App, movement: Move) {
    let buffer = &mut app
        .surfaces
        .llm_answer
        .as_mut()
        .expect("answer active")
        .buffer;
    match movement {
        Move::Left => buffer.move_left(),
        Move::Right => buffer.move_right(),
        Move::Up => buffer.move_up(),
        Move::Down => buffer.move_down(),
    }
}

fn move_page(app: &mut super::App, forward: bool) {
    for _ in 0..app.screen.visible_height().max(1) {
        move_cursor(app, if forward { Move::Down } else { Move::Up });
    }
}

fn set_line_edge(app: &mut super::App, end: bool) {
    let buffer = &mut app
        .surfaces
        .llm_answer
        .as_mut()
        .expect("answer active")
        .buffer;
    let row = buffer.cursor().row;
    let col = if end {
        buffer.line_char_count(row).unwrap_or(0)
    } else {
        0
    };
    buffer.set_cursor(Cursor { row, col });
}

fn reveal_cursor(app: &mut super::App) {
    let cursor = app
        .surfaces
        .llm_answer
        .as_ref()
        .expect("answer active")
        .buffer
        .cursor();
    app.screen.reveal_row(cursor.row);
    app.screen
        .reveal_col_with_width(cursor.col, super::view::content_width(app));
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests;
