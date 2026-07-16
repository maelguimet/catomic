//! Purpose: this file must preview and explicitly confirm validated LLM patches.
//! Owns: transient patch view state, confirmation keys, stale-source checks, and apply.
//! Must not: construct clients, call endpoints, read repos, write files, or auto-apply.
//! Invariants: Enter is the only apply action; apply is one undoable buffer transaction.
//! Phase: 6 (LLM, Powerful but Caged).

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Buffer, Cursor, PieceTable};
use crate::llm::patch::Patch;

pub(crate) struct PatchPreview {
    patch: Patch,
    proposed_text: String,
    buffer: PieceTable,
    source_scroll_top: usize,
    source_scroll_left: usize,
}

pub(crate) fn show(app: &mut super::App, out: &mut dyn Write, patch_text: &str) -> io::Result<()> {
    if app.buffer.is_read_only() || app.buffer.page_info().is_some() {
        app.message =
            Some("LLM patch preview requires a fully editable current buffer.".to_string());
        return app.render(out);
    }
    let (patch, proposed_text) = match build_proposal(&app.buffer.to_string(), patch_text) {
        Ok(proposal) => proposal,
        Err(message) => {
            app.message = Some(message);
            return app.render(out);
        }
    };

    super::view::cancel_preview(app);
    super::lint::close_view(app);
    super::project_files::close_view(app);
    close(app);
    app.llm_preview = Some(PatchPreview {
        patch,
        proposed_text,
        buffer: PieceTable::from_text(patch_text),
        source_scroll_top: app.screen.scroll_top,
        source_scroll_left: app.screen.scroll_left,
    });
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    app.selection.clear();
    app.message = Some("LLM patch preview (read-only). Enter applies; Esc cancels.".to_string());
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
    match key.code {
        KeyCode::Enter => confirm(app, out)?,
        KeyCode::Esc => cancel(app, out)?,
        KeyCode::Left => move_cursor(app, Move::Left),
        KeyCode::Right => move_cursor(app, Move::Right),
        KeyCode::Up => move_cursor(app, Move::Up),
        KeyCode::Down => move_cursor(app, Move::Down),
        KeyCode::PageUp => move_page(app, false),
        KeyCode::PageDown => move_page(app, true),
        KeyCode::Home => set_line_edge(app, false),
        KeyCode::End => set_line_edge(app, true),
        _ => {
            app.message =
                Some("LLM patch preview is read-only. Enter applies; Esc cancels.".to_string())
        }
    }
    if is_viewing(app) {
        reveal_preview_cursor(app);
        app.render(out)?;
    }
    Ok(true)
}

pub(crate) fn handle_paste(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    if !is_viewing(app) {
        return Ok(false);
    }
    app.message = Some("LLM patch preview is read-only. Enter applies; Esc cancels.".to_string());
    app.render(out)?;
    Ok(true)
}

pub(crate) fn is_viewing(app: &super::App) -> bool {
    app.llm_preview.is_some()
}

pub(crate) fn display_buffer(app: &super::App) -> Option<&dyn Buffer> {
    app.llm_preview
        .as_ref()
        .map(|preview| &preview.buffer as &dyn Buffer)
}

pub(crate) fn close(app: &mut super::App) -> bool {
    if let Some(preview) = app.llm_preview.take() {
        app.screen.scroll_top = preview.source_scroll_top;
        app.screen.scroll_left = preview.source_scroll_left;
        true
    } else {
        false
    }
}

fn confirm(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let preview = app.llm_preview.take().expect("preview active");
    app.screen.scroll_top = preview.source_scroll_top;
    app.screen.scroll_left = preview.source_scroll_left;
    let current = app.buffer.to_string();
    if preview.patch.apply_preview(&current).ok().as_ref() != Some(&preview.proposed_text) {
        app.message = Some("Source changed since preview; LLM patch was not applied.".to_string());
        app.reveal_cursor();
        return app.render(out);
    }
    if current == preview.proposed_text {
        app.message = Some("LLM patch makes no changes.".to_string());
        app.reveal_cursor();
        return app.render(out);
    }
    if !replace_whole_buffer(&mut *app.buffer, &preview.proposed_text)? {
        app.message = Some("Current buffer refused the LLM patch.".to_string());
        return app.render(out);
    }
    super::input::finish_content_edit_with_message(
        app,
        out,
        Some("LLM patch applied; Ctrl+Z undoes it.".to_string()),
    )
}

fn build_proposal(current: &str, patch_text: &str) -> Result<(Patch, String), String> {
    let patch =
        Patch::parse(patch_text).map_err(|error| format!("Invalid LLM patch: {error:?}"))?;
    let proposed_text = patch
        .apply_preview(current)
        .map_err(|error| format!("LLM patch does not match current text: {error:?}"))?;
    Ok((patch, proposed_text))
}

fn replace_whole_buffer(buffer: &mut dyn Buffer, text: &str) -> io::Result<bool> {
    let end_row = buffer.line_count().saturating_sub(1);
    let end = Cursor {
        row: end_row,
        col: buffer.line_char_count(end_row).unwrap_or(0),
    };
    buffer.replace_range(Cursor::default(), end, text)
}

fn cancel(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    close(app);
    app.message = Some("LLM patch cancelled; no changes applied.".to_string());
    app.reveal_cursor();
    app.render(out)
}

enum Move {
    Left,
    Right,
    Up,
    Down,
}

fn move_cursor(app: &mut super::App, movement: Move) {
    let buffer = &mut app.llm_preview.as_mut().expect("preview active").buffer;
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
    let buffer = &mut app.llm_preview.as_mut().expect("preview active").buffer;
    let row = buffer.cursor().row;
    let col = end
        .then(|| buffer.line_char_count(row).unwrap_or(0))
        .unwrap_or(0);
    buffer.set_cursor(Cursor { row, col });
}

fn reveal_preview_cursor(app: &mut super::App) {
    let cursor = app
        .llm_preview
        .as_ref()
        .expect("preview active")
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
