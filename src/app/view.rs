//! Purpose: own non-mutating per-buffer display modes and their key bindings.
//! Owns: F6 preview, F7/F8 indicators, F9 soft wrap, and display coordinates.
//! Must not: mutate source text/history, read files, emit terminal setup, or network.
//! Invariants: preview is explicit/read-only; toggles and source viewport are per buffer.
//! Phase: 4-b/4-c optional indicators and Markdown preview.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Buffer, Cursor, PieceTable};
use crate::editor::syntax::{self, SyntaxKind};

#[derive(Debug, Default)]
pub(crate) struct ViewOptions {
    pub(crate) line_numbers: bool,
    pub(crate) whitespace: bool,
    pub(crate) soft_wrap: bool,
    preview: Option<PreviewDocument>,
}

#[derive(Debug)]
struct PreviewDocument {
    buffer: PieceTable,
    source_scroll_top: usize,
    source_scroll_left: usize,
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    match key.code {
        KeyCode::F(6) => return toggle_preview(app, out),
        KeyCode::F(7) => return toggle_indicator(app, out, true),
        KeyCode::F(8) => return toggle_indicator(app, out, false),
        KeyCode::F(9) => return toggle_soft_wrap(app, out),
        _ => {}
    }
    if !is_preview(app) || is_quit(key) {
        return Ok(false);
    }
    handle_preview_key(app, out, key)?;
    Ok(true)
}

pub(crate) fn handle_paste(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    if !is_preview(app) {
        return Ok(false);
    }
    read_only_message(app);
    app.render(out)?;
    Ok(true)
}

pub(crate) fn is_preview(app: &super::App) -> bool {
    app.view.preview.is_some()
}

pub(crate) fn display_buffer(app: &super::App) -> &dyn Buffer {
    if let Some(buffer) = super::help::display_buffer(app) {
        return buffer;
    }
    if let Some(buffer) = super::recovery::display_buffer(app) {
        return buffer;
    }
    if let Some(buffer) = super::external_command::display_buffer(app) {
        return buffer;
    }
    if let Some(buffer) = super::llm_preview::display_buffer(app) {
        return buffer;
    }
    if let Some(buffer) = super::llm_answer::display_buffer(app) {
        return buffer;
    }
    if let Some(buffer) = super::project_files::display_buffer(app) {
        return buffer;
    }
    if let Some(buffer) = super::lint::display_buffer(app) {
        return buffer;
    }
    app.view
        .preview
        .as_ref()
        .map(|preview| &preview.buffer as &dyn Buffer)
        .unwrap_or(&*app.buffer)
}

pub(crate) fn source_is_displayed(app: &super::App) -> bool {
    // Mouse coordinates are only valid for the source when the rendered trait
    // object is that exact buffer, not one of the read-only overlay buffers.
    let source: &dyn Buffer = &*app.buffer;
    std::ptr::eq(display_buffer(app), source)
}

pub(crate) fn display_syntax(app: &super::App) -> SyntaxKind {
    if super::help::is_viewing(app)
        || super::recovery::is_viewing(app)
        || super::external_command::is_viewing(app)
        || super::llm_preview::is_viewing(app)
        || super::llm_answer::is_viewing(app)
        || super::lint::is_viewing(app)
        || super::project_files::is_viewing(app)
    {
        SyntaxKind::Plain
    } else if is_preview(app) {
        SyntaxKind::MarkdownPreview
    } else {
        syntax::syntax_for_path(app.file.path.as_deref())
    }
}

pub(crate) fn gutter_width(app: &super::App) -> usize {
    if app.view.line_numbers {
        crate::terminal::render::line_number_gutter(display_buffer(app).line_count())
    } else {
        0
    }
}

pub(crate) fn content_width(app: &super::App) -> usize {
    (app.screen.width as usize).saturating_sub(gutter_width(app))
}

pub(crate) fn soft_wrap_active(app: &super::App) -> bool {
    app.view.soft_wrap
        && !is_preview(app)
        && !super::help::is_viewing(app)
        && !super::recovery::is_viewing(app)
        && !super::external_command::is_viewing(app)
        && !super::llm_preview::is_viewing(app)
        && !super::llm_answer::is_viewing(app)
        && !super::lint::is_viewing(app)
        && !super::project_files::is_viewing(app)
}

pub(crate) fn cancel_preview(app: &mut super::App) {
    if let Some(preview) = app.view.preview.take() {
        app.screen.scroll_top = preview.source_scroll_top;
        app.screen.scroll_left = preview.source_scroll_left;
    }
}

fn toggle_preview(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    if is_preview(app) {
        cancel_preview(app);
        app.message = Some("Markdown preview off.".to_string());
        app.reveal_cursor();
    } else if syntax::syntax_for_path(app.file.path.as_deref()) != SyntaxKind::Markdown {
        app.message = Some("Markdown preview is available for .md files.".to_string());
    } else {
        let rendered = crate::editor::markdown_preview::render(&app.buffer.to_string());
        app.view.preview = Some(PreviewDocument {
            buffer: PieceTable::from_text(&rendered),
            source_scroll_top: app.screen.scroll_top,
            source_scroll_left: app.screen.scroll_left,
        });
        app.screen.scroll_top = 0;
        app.screen.scroll_left = 0;
        app.selection.clear();
        app.message = Some("Markdown preview on (read-only; F6 or Esc to exit).".to_string());
    }
    reveal_display_cursor(app);
    app.render(out)?;
    Ok(true)
}

fn toggle_indicator(
    app: &mut super::App,
    out: &mut dyn Write,
    line_numbers: bool,
) -> io::Result<bool> {
    let (label, enabled) = if line_numbers {
        app.view.line_numbers = !app.view.line_numbers;
        ("Line numbers", app.view.line_numbers)
    } else {
        app.view.whitespace = !app.view.whitespace;
        ("Whitespace indicators", app.view.whitespace)
    };
    app.message = Some(format!("{label} {}.", if enabled { "on" } else { "off" }));
    reveal_display_cursor(app);
    app.render(out)?;
    Ok(true)
}

fn toggle_soft_wrap(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    app.view.soft_wrap = !app.view.soft_wrap;
    app.screen.scroll_left = 0;
    app.screen.wrap_col = 0;
    app.message = Some(format!(
        "Soft wrap {}.",
        if app.view.soft_wrap { "on" } else { "off" }
    ));
    app.reveal_cursor();
    app.render(out)?;
    Ok(true)
}

fn handle_preview_key(app: &mut super::App, out: &mut dyn Write, key: KeyEvent) -> io::Result<()> {
    if key.code == KeyCode::Esc {
        cancel_preview(app);
        app.message = Some("Markdown preview off.".to_string());
        app.reveal_cursor();
        return app.render(out);
    }
    let height = app.screen.visible_height().max(1);
    let preview = &mut app.view.preview.as_mut().expect("preview active").buffer;
    match key.code {
        KeyCode::Left => preview.move_left(),
        KeyCode::Right => preview.move_right(),
        KeyCode::Up => preview.move_up(),
        KeyCode::Down => preview.move_down(),
        KeyCode::PageUp => move_rows(preview, false, height),
        KeyCode::PageDown => move_rows(preview, true, height),
        KeyCode::Home => preview.set_cursor(Cursor {
            row: preview.cursor().row,
            col: 0,
        }),
        KeyCode::End => preview.set_cursor(Cursor {
            row: preview.cursor().row,
            col: preview.line_char_count(preview.cursor().row).unwrap_or(0),
        }),
        _ => read_only_message(app),
    }
    reveal_display_cursor(app);
    app.render(out)
}

fn move_rows(buffer: &mut PieceTable, forward: bool, count: usize) {
    for _ in 0..count {
        if forward {
            buffer.move_down();
        } else {
            buffer.move_up();
        }
    }
}

fn reveal_display_cursor(app: &mut super::App) {
    app.screen.clamp_scroll();
    super::viewport::clamp_viewport_to_buffer(app);
    let cursor = display_buffer(app).cursor();
    app.screen.reveal_row(cursor.row);
    app.screen
        .reveal_col_with_width(cursor.col, content_width(app));
    super::viewport::clamp_viewport_to_buffer(app);
}

fn read_only_message(app: &mut super::App) {
    app.message = Some("Markdown preview is read-only; press F6 or Esc to edit.".to_string());
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn function_keys_toggle_view_state_and_render_indicators() {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("a b\tc"));
        let mut out = Vec::new();

        handle_key(&mut app, &mut out, key(KeyCode::F(7))).unwrap();
        assert!(app.view.line_numbers);
        assert!(String::from_utf8_lossy(&out).contains("1 "));

        out.clear();
        handle_key(&mut app, &mut out, key(KeyCode::F(8))).unwrap();
        assert!(app.view.whitespace);
        assert!(String::from_utf8(out).unwrap().contains("a·b→c"));
    }

    #[test]
    fn f9_toggles_bounded_soft_wrapping() {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("abcdef"));
        app.screen.width = 3;
        app.screen.height = 4;
        let mut out = Vec::new();

        handle_key(&mut app, &mut out, key(KeyCode::F(9))).unwrap();
        assert!(app.view.soft_wrap);
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("\x1b[1;1H\x1b[Kabc"));
        assert!(rendered.contains("\x1b[2;1H\x1b[Kdef"));

        let mut out = Vec::new();
        handle_key(&mut app, &mut out, key(KeyCode::F(9))).unwrap();
        assert!(!app.view.soft_wrap);
    }

    #[test]
    fn markdown_preview_is_rendered_read_only_and_restores_source_view() {
        let mut app = super::super::App::new(None).unwrap();
        app.file.path = Some(PathBuf::from("notes.md"));
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("# Title\n\n- item"));
        app.screen.width = 4;
        app.buffer.set_cursor(Cursor { row: 0, col: 7 });
        app.reveal_cursor();
        let source_scroll_left = app.screen.scroll_left;
        let mut out = Vec::new();

        handle_key(&mut app, &mut out, key(KeyCode::F(6))).unwrap();
        assert!(is_preview(&app));
        assert!(String::from_utf8_lossy(&out).contains('▌'));
        let source = app.buffer.to_string();

        handle_key(&mut app, &mut out, key(KeyCode::Char('x'))).unwrap();
        assert_eq!(app.buffer.to_string(), source);
        assert!(app.message.as_deref().unwrap().contains("read-only"));

        handle_key(&mut app, &mut out, key(KeyCode::F(6))).unwrap();
        assert!(!is_preview(&app));
        assert_eq!(app.screen.scroll_left, source_scroll_left);
    }

    #[test]
    fn preview_rejects_non_markdown_files() {
        let mut app = super::super::App::new(None).unwrap();
        app.file.path = Some(PathBuf::from("notes.txt"));
        let mut out = Vec::new();

        handle_key(&mut app, &mut out, key(KeyCode::F(6))).unwrap();

        assert!(!is_preview(&app));
        assert!(app.message.as_deref().unwrap().contains(".md files"));
    }

    #[test]
    fn bracketed_paste_cannot_mutate_a_previewed_source() {
        let mut app = super::super::App::new(None).unwrap();
        app.file.path = Some(PathBuf::from("notes.md"));
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("# Original"));
        let mut out = Vec::new();
        handle_key(&mut app, &mut out, key(KeyCode::F(6))).unwrap();

        super::super::input::handle_paste(&mut app, &mut out, "replacement").unwrap();

        assert_eq!(app.buffer.to_string(), "# Original");
        assert!(app.message.as_deref().unwrap().contains("read-only"));
    }
}
