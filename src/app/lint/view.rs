//! Purpose: present Project diagnostics as a transient read-only document.
//! Owns: list formatting, display-buffer selection, navigation, paste guard, and restoration.
//! Must not: run linters, load config, mutate source/history, scan projects, or network.
//! Invariants: Escape restores the source viewport; all non-navigation input is read-only.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Buffer, Cursor, PieceTable};
use crate::project::diagnostics::Severity;

pub(crate) struct DiagnosticsView {
    buffer: PieceTable,
    source_scroll_top: usize,
    source_scroll_left: usize,
}

pub(crate) fn show_diagnostics(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    super::super::project_files::close_view(app);
    let Some(project) = app.project.as_ref() else {
        app.message_info("Diagnostics require Project mode (:project).");
        return app.render(out);
    };
    if project.diagnostics().items.is_empty() {
        app.message_info("No diagnostics to show; run :lint first.");
        return app.render(out);
    }
    let mut text = String::new();
    for (index, item) in project.diagnostics().items.iter().enumerate() {
        let severity = match item.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        text.push_str(&format!(
            "{}. {severity} {}:{}:{} {}\n",
            index + 1,
            item.file.display(),
            item.line,
            item.col,
            item.message
        ));
    }
    super::super::view::cancel_preview(app);
    app.surfaces.diagnostics = Some(DiagnosticsView {
        buffer: PieceTable::from_owned_text(text),
        source_scroll_top: app.screen.scroll_top,
        source_scroll_left: app.screen.scroll_left,
    });
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    app.message_info("Diagnostics (read-only; arrows move, Esc closes).");
    app.render(out)
}

pub(crate) fn handle_key(
    app: &mut super::super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if is_viewing(app) {
        if is_quit(key) {
            return Ok(false);
        }
        handle_view_key(app, out, key)?;
        return Ok(true);
    }
    if key.code == KeyCode::Esc
        && app
            .project
            .as_mut()
            .is_some_and(|project| project.cancel_linter())
    {
        app.message = None;
        app.render(out)?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) fn handle_paste(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<bool> {
    if !is_viewing(app) {
        return Ok(false);
    }
    app.message_info("Diagnostics view is read-only; press Esc to close.");
    app.render(out)?;
    Ok(true)
}

pub(crate) fn is_viewing(app: &super::super::App) -> bool {
    app.surfaces.diagnostics.is_some()
}

pub(crate) fn display_buffer(app: &super::super::App) -> Option<&dyn Buffer> {
    app.surfaces
        .diagnostics
        .as_ref()
        .map(|view| &view.buffer as &dyn Buffer)
}

pub(crate) fn close_view(app: &mut super::super::App) {
    if let Some(view) = app.surfaces.diagnostics.take() {
        app.screen.scroll_top = view.source_scroll_top;
        app.screen.scroll_left = view.source_scroll_left;
    }
}

fn handle_view_key(
    app: &mut super::super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<()> {
    if key.code == KeyCode::Esc {
        close_view(app);
        app.message = None;
        app.reveal_cursor();
        return app.render(out);
    }
    let rows = app.screen.visible_height().max(1);
    let buffer = &mut app
        .surfaces
        .diagnostics
        .as_mut()
        .expect("view active")
        .buffer;
    match key.code {
        KeyCode::Left => buffer.move_left(),
        KeyCode::Right => buffer.move_right(),
        KeyCode::Up => buffer.move_up(),
        KeyCode::Down => buffer.move_down(),
        KeyCode::PageUp => move_view_rows(buffer, false, rows),
        KeyCode::PageDown => move_view_rows(buffer, true, rows),
        KeyCode::Home => buffer.set_cursor(Cursor {
            row: buffer.cursor().row,
            col: 0,
        }),
        KeyCode::End => buffer.set_cursor(Cursor {
            row: buffer.cursor().row,
            col: buffer.line_char_count(buffer.cursor().row).unwrap_or(0),
        }),
        _ => app.message_info("Diagnostics view is read-only; press Esc to close."),
    }
    app.reveal_cursor();
    app.render(out)
}

fn move_view_rows(buffer: &mut PieceTable, forward: bool, rows: usize) {
    for _ in 0..rows {
        if forward {
            buffer.move_down();
        } else {
            buffer.move_up();
        }
    }
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}
