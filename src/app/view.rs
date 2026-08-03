//! Purpose: own non-mutating display modes and their key bindings.
//! Owns: F5 external changes, F6 preview, F7/F8 indicators, F9 wrap, and coordinates.
//! Must not: mutate source text/history, emit terminal setup, or contact the network.
//! Invariants: preview is explicit/read-only; F5/F7 are session-global and persisted;
//!   F8/F9 remain per buffer; Markdown preview synchronizes its vertical viewport with source.

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Buffer, Cursor, PreviewBuffer};
use crate::config::actions::Action;
use crate::editor::markdown_preview::MarkdownAnnotations;
use crate::editor::syntax::{self, SyntaxKind};

#[derive(Debug, Default)]
pub(crate) struct ViewOptions {
    pub(crate) whitespace: bool,
    pub(crate) soft_wrap: bool,
    preview: Option<PreviewDocument>,
}

#[derive(Debug)]
struct PreviewDocument {
    buffer: PreviewBuffer,
    annotations: MarkdownAnnotations,
    layout_width: usize,
    source_scroll_top: usize,
    initial_preview_scroll_top: usize,
    source_scroll_left: usize,
    source_wrap_col: usize,
}

pub(crate) fn display_presentation(
    app: &super::App,
) -> Option<crate::terminal::render::DocumentPresentation<'_>> {
    if let Some(presentation) = super::help::presentation(app) {
        return Some(presentation);
    }
    app.view
        .preview
        .as_ref()
        .map(|preview| crate::terminal::render::DocumentPresentation {
            annotations: &preview.annotations,
        })
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    key: KeyEvent,
) -> io::Result<bool> {
    if !is_preview(app) || is_quit(key) {
        return Ok(false);
    }
    handle_preview_key(app, out, key)?;
    Ok(true)
}

pub(crate) fn dispatch_action(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    action: Action,
) -> io::Result<bool> {
    match action {
        Action::ToggleExternalDiff => toggle_external_diff(app, out),
        Action::MarkdownPreview => toggle_preview(app, out),
        Action::LineNumbers => toggle_line_numbers(app, out),
        Action::Whitespace => toggle_whitespace(app, out),
        Action::SoftWrap => toggle_soft_wrap(app, out),
        _ => Ok(false),
    }
}

pub(crate) fn dispatch_preview_action(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    action: Action,
) -> io::Result<bool> {
    if !is_preview(app) {
        return Ok(false);
    }
    if matches!(action, Action::MarkdownPreview | Action::PreviewCancel) {
        cancel_preview(app);
        app.message = None;
        app.render(out)?;
        return Ok(true);
    }
    let height = app.screen.visible_height().max(1);
    let preview = &mut app.view.preview.as_mut().expect("preview active").buffer;
    match action {
        Action::MoveLeft => preview.move_left(),
        Action::MoveRight => preview.move_right(),
        Action::MoveUp => preview.move_up(),
        Action::MoveDown => preview.move_down(),
        Action::ViewportUp => move_rows(preview, false, height),
        Action::ViewportDown => move_rows(preview, true, height),
        Action::LineStart => preview.set_cursor(Cursor {
            row: preview.cursor().row,
            col: 0,
        }),
        Action::LineEnd => preview.set_cursor(Cursor {
            row: preview.cursor().row,
            col: preview.line_char_count(preview.cursor().row).unwrap_or(0),
        }),
        Action::PreviewAccept => {}
        _ => return Ok(false),
    }
    reveal_display_cursor(app);
    app.render(out)?;
    Ok(true)
}

pub(crate) fn handle_paste(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<bool> {
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
    if let Some(buffer) = super::mobile::display_buffer(app) {
        return buffer;
    }
    if let Some(buffer) = super::help::display_buffer(app) {
        return buffer;
    }
    if let Some(buffer) = super::recovery::display_buffer(app) {
        return buffer;
    }
    if let Some(buffer) = super::external_command::display_buffer(app) {
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
    if super::help::is_viewing(app) {
        SyntaxKind::MarkdownPreview
    } else if super::mobile::is_viewing(app)
        || super::recovery::is_viewing(app)
        || super::external_command::is_viewing(app)
    {
        SyntaxKind::Plain
    } else if is_preview(app) {
        SyntaxKind::MarkdownPreview
    } else {
        syntax::syntax_for_path(app.file.path.as_deref())
    }
}

pub(crate) fn display_surface(app: &super::App) -> crate::terminal::render::ContentSurface {
    use crate::terminal::render::ContentSurface;
    if super::mobile::is_viewing(app)
        || super::help::is_viewing(app)
        || super::recovery::is_viewing(app)
        || super::external_command::is_viewing(app)
        || is_preview(app)
    {
        ContentSurface::Preview
    } else {
        ContentSurface::Normal
    }
}

pub(crate) fn gutter_width(app: &super::App) -> usize {
    if super::mobile::is_viewing(app) {
        return 0;
    }
    let line_numbers = if app.view_preferences.line_numbers() {
        crate::terminal::render::line_number_gutter(display_buffer(app).line_count())
    } else {
        0
    };
    let source_is_visible = source_is_displayed(app);
    let external_changes = (source_is_visible && app.view_preferences.external_diff())
        .then(|| app.external_changes.visible(app.buffer.content_revision()))
        .flatten();
    line_numbers
        + crate::terminal::render::change_gutter_width(
            external_changes.is_some_and(|changes| !changes.markers.is_empty()),
        )
}

pub(crate) fn content_width(app: &super::App) -> usize {
    (app.screen.width as usize).saturating_sub(gutter_width(app))
}

pub(crate) fn soft_wrap_active(app: &super::App) -> bool {
    !super::mobile::is_viewing(app)
        && (super::help::is_viewing(app)
            || (app.view.soft_wrap
                && !is_preview(app)
                && !super::recovery::is_viewing(app)
                && !super::external_command::is_viewing(app)))
}

pub(crate) fn cancel_preview(app: &mut super::App) {
    if let Some(preview) = app.view.preview.take() {
        app.screen.scroll_top = if app.screen.scroll_top == preview.initial_preview_scroll_top {
            preview.source_scroll_top
        } else {
            map_viewport_top(
                app.screen.scroll_top,
                preview.buffer.line_count(),
                app.buffer.line_count(),
                app.screen.visible_height(),
            )
        };
        app.screen.scroll_left = preview.source_scroll_left;
        app.screen.wrap_col = preview.source_wrap_col;
    }
}

pub(crate) fn relayout_preview(app: &mut super::App) {
    let Some(preview) = app.view.preview.as_ref() else {
        return;
    };
    let width = crate::editor::markdown_preview::layout_width(content_width(app));
    if preview.layout_width == width {
        return;
    }
    let cursor = preview.buffer.cursor();
    let old_line_count = preview.buffer.line_count();
    let old_scroll_top = app.screen.scroll_top;
    let old_initial_scroll_top = preview.initial_preview_scroll_top;
    let viewport_was_untouched = old_scroll_top == old_initial_scroll_top;
    let visible_height = app.screen.visible_height();
    match crate::editor::markdown_preview::render_with_width(&app.buffer.to_string(), width) {
        Ok(rendered) => {
            let (mut buffer, annotations) = rendered.into_buffer_and_annotations();
            let new_line_count = buffer.line_count();
            let new_initial_scroll_top = map_viewport_top(
                old_initial_scroll_top,
                old_line_count,
                new_line_count,
                visible_height,
            );
            let new_scroll_top = if viewport_was_untouched {
                new_initial_scroll_top
            } else {
                map_viewport_top(
                    old_scroll_top,
                    old_line_count,
                    new_line_count,
                    visible_height,
                )
            };
            let row = if viewport_was_untouched {
                new_initial_scroll_top
            } else {
                cursor.row
            }
            .min(buffer.line_count().saturating_sub(1));
            let col = cursor.col.min(buffer.line_char_count(row).unwrap_or(0));
            buffer.set_cursor(Cursor { row, col });
            if let Some(preview) = app.view.preview.as_mut() {
                preview.buffer = buffer;
                preview.annotations = annotations;
                preview.layout_width = width;
                preview.initial_preview_scroll_top = new_initial_scroll_top;
                app.screen.scroll_top = new_scroll_top;
                app.screen.scroll_left = 0;
            }
        }
        Err(error) => app.message_error(format!("Markdown preview failed: {error}.")),
    }
}

fn toggle_preview(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<bool> {
    if is_preview(app) {
        cancel_preview(app);
        app.message = None;
    } else {
        let width = crate::editor::markdown_preview::layout_width(content_width(app));
        match crate::editor::markdown_preview::render_with_width(&app.buffer.to_string(), width) {
            Ok(rendered) => {
                let source_scroll_top = app.screen.scroll_top;
                let (mut buffer, annotations) = rendered.into_buffer_and_annotations();
                let initial_preview_scroll_top = map_viewport_top(
                    source_scroll_top,
                    app.buffer.line_count(),
                    buffer.line_count(),
                    app.screen.visible_height(),
                );
                let cursor_row =
                    initial_preview_scroll_top.min(buffer.line_count().saturating_sub(1));
                buffer.set_cursor(Cursor {
                    row: cursor_row,
                    col: 0,
                });
                app.view.preview = Some(PreviewDocument {
                    buffer,
                    annotations,
                    layout_width: width,
                    source_scroll_top,
                    initial_preview_scroll_top,
                    source_scroll_left: app.screen.scroll_left,
                    source_wrap_col: app.screen.wrap_col,
                });
                app.screen.scroll_top = initial_preview_scroll_top;
                app.screen.scroll_left = 0;
                app.screen.wrap_col = 0;
                app.message_info("Markdown preview on (read-only; F6 or Esc to exit).");
                reveal_display_cursor(app);
            }
            Err(error) => app.message_error(format!("Markdown preview failed: {error}.")),
        }
    }
    app.render(out)?;
    Ok(true)
}

fn map_viewport_top(
    scroll_top: usize,
    from_line_count: usize,
    to_line_count: usize,
    visible_height: usize,
) -> usize {
    if visible_height == 0 {
        return 0;
    }
    let from_maximum = from_line_count.saturating_sub(visible_height);
    let to_maximum = to_line_count.saturating_sub(visible_height);
    if from_maximum == 0 || to_maximum == 0 {
        return 0;
    }
    let position = scroll_top.min(from_maximum) as u128;
    let numerator = position
        .saturating_mul(to_maximum as u128)
        .saturating_add((from_maximum / 2) as u128);
    (numerator / from_maximum as u128) as usize
}

fn toggle_line_numbers(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<bool> {
    let enabled = !app.view_preferences.line_numbers();
    app.view_preferences.set_line_numbers(enabled);
    match app.view_preferences.persist() {
        Ok(()) => app.message = None,
        Err(error) => app.message_error(format!("Line-number preference not saved: {error}.")),
    }
    relayout_preview(app);
    reveal_display_cursor(app);
    app.render(out)?;
    Ok(true)
}

fn toggle_external_diff(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<bool> {
    let enabled = !app.view_preferences.external_diff();
    app.view_preferences.set_external_diff(enabled);
    if !enabled {
        app.clear_external_changes();
    }
    match app.view_preferences.persist() {
        Ok(()) => app.message = None,
        Err(error) => app.message_error(format!("External-change preference not saved: {error}.")),
    }
    reveal_display_cursor(app);
    app.render(out)?;
    Ok(true)
}

fn toggle_whitespace(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<bool> {
    app.view.whitespace = !app.view.whitespace;
    app.message = None;
    reveal_display_cursor(app);
    app.render(out)?;
    Ok(true)
}

fn toggle_soft_wrap(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<bool> {
    app.view.soft_wrap = !app.view.soft_wrap;
    app.screen.scroll_left = 0;
    app.screen.wrap_col = 0;
    app.message = None;
    app.reveal_cursor();
    app.render(out)?;
    Ok(true)
}

fn handle_preview_key(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    key: KeyEvent,
) -> io::Result<()> {
    if key.code == KeyCode::Esc {
        cancel_preview(app);
        app.message = None;
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

fn move_rows(buffer: &mut dyn Buffer, forward: bool, count: usize) {
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
    app.message_info("Markdown preview is read-only; press F6 or Esc to edit.");
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

    fn press(
        app: &mut super::super::App,
        out: &mut dyn crate::terminal::TerminalOutput,
        code: KeyCode,
    ) {
        app.handle_key_with(out, key(code)).unwrap();
    }

    #[test]
    fn function_keys_toggle_view_state_and_render_indicators() {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("a b\tc"));
        let mut out = Vec::new();

        press(&mut app, &mut out, KeyCode::F(7));
        assert!(app.view_preferences.line_numbers());
        assert!(app
            .message
            .as_deref()
            .unwrap()
            .contains("preference not saved"));
        assert!(String::from_utf8_lossy(&out).contains("1 "));

        out.clear();
        press(&mut app, &mut out, KeyCode::F(8));
        assert!(app.view.whitespace);
        assert!(String::from_utf8(out).unwrap().contains("a·b→c"));
    }

    #[test]
    fn f5_toggles_only_external_diff_presentation_and_dismisses_current_marks() {
        let mut app = super::super::App::new(None).unwrap();
        let old = crate::buffer::PieceTable::from_text("before");
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("after"));
        app.external_changes = match super::super::external_diff::compare(&old, &*app.buffer) {
            super::super::external_diff::DiffOutcome::Compared(changes) => changes,
            super::super::external_diff::DiffOutcome::Skipped(reason) => {
                panic!("unexpected skip: {reason}")
            }
        };
        let text = app.buffer.to_string();
        let history = app.buffer.edit_history_position();
        let dirty = app.file.dirty;
        let mut out = Vec::new();

        press(&mut app, &mut out, KeyCode::F(5));

        assert!(!app.view_preferences.external_diff());
        assert!(app.external_changes.visible(history).is_none());
        assert_eq!(app.buffer.to_string(), text);
        assert_eq!(app.buffer.edit_history_position(), history);
        assert_eq!(app.file.dirty, dirty);

        press(&mut app, &mut out, KeyCode::F(5));
        assert!(app.view_preferences.external_diff());
        assert!(app.external_changes.visible(history).is_none());
    }

    #[test]
    fn f9_toggles_bounded_soft_wrapping() {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("abcdef"));
        app.screen.width = 3;
        app.screen.height = 4;
        let mut out = Vec::new();

        press(&mut app, &mut out, KeyCode::F(9));
        assert!(app.view.soft_wrap);
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("\x1b[1;1H\x1b[Kabc"));
        assert!(rendered.contains("\x1b[2;1H\x1b[Kdef"));

        let mut out = Vec::new();
        press(&mut app, &mut out, KeyCode::F(9));
        assert!(!app.view.soft_wrap);
    }

    #[test]
    fn markdown_preview_is_rendered_read_only_and_restores_source_view() {
        let mut app = super::super::App::new(None).unwrap();
        app.file.path = Some(PathBuf::from("notes.txt"));
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("# Title\n\n- item"));
        app.screen.width = 6;
        app.buffer.set_cursor(Cursor { row: 0, col: 7 });
        app.reveal_cursor();
        let source_scroll_left = app.screen.scroll_left;
        let mut out = Vec::new();

        press(&mut app, &mut out, KeyCode::F(6));
        assert!(is_preview(&app));
        assert!(String::from_utf8_lossy(&out).contains("Titl"));
        assert!(!String::from_utf8_lossy(&out).contains("# Title"));
        let source = app.buffer.to_string();

        press(&mut app, &mut out, KeyCode::Char('x'));
        assert_eq!(app.buffer.to_string(), source);
        assert!(app.message.as_deref().unwrap().contains("read-only"));

        press(&mut app, &mut out, KeyCode::F(6));
        assert!(!is_preview(&app));
        assert_eq!(app.screen.scroll_left, source_scroll_left);
    }

    #[test]
    fn preview_accepts_every_filename_and_untitled_buffers() {
        for path in [None, Some("README"), Some("notes.txt"), Some("notes.md")] {
            let mut app = super::super::App::new(None).unwrap();
            app.file.path = path.map(PathBuf::from);
            app.buffer = Box::new(crate::buffer::PieceTable::from_text("# Preview"));
            let mut out = Vec::new();

            press(&mut app, &mut out, KeyCode::F(6));

            assert!(is_preview(&app), "path {path:?}");
            assert!(!app.message.as_deref().unwrap().contains(".md"));
        }
    }

    #[test]
    fn bracketed_paste_cannot_mutate_a_previewed_source() {
        let mut app = super::super::App::new(None).unwrap();
        app.file.path = Some(PathBuf::from("notes.md"));
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("# Original"));
        let mut out = Vec::new();
        press(&mut app, &mut out, KeyCode::F(6));

        super::super::input::handle_paste(&mut app, &mut out, "replacement").unwrap();

        assert_eq!(app.buffer.to_string(), "# Original");
        assert!(app.message.as_deref().unwrap().contains("read-only"));
    }

    #[test]
    fn narrow_table_preview_uses_a_wrapped_borderless_fallback() {
        let mut app = super::super::App::new(None).unwrap();
        app.file.path = Some(PathBuf::from("table.md"));
        app.buffer = Box::new(crate::buffer::PieceTable::from_text(
            "| Left | Center | Right |\n| :--- | :----: | ----: |\n| wide 猫 emoji 🐾 | a much longer value | 2,000 |",
        ));
        app.screen.width = 44;
        app.screen.height = 14;
        let mut out = Vec::new();

        press(&mut app, &mut out, KeyCode::F(6));
        let preview_text = app.view.preview.as_ref().unwrap().buffer.to_string();
        out.clear();
        press(&mut app, &mut out, KeyCode::End);

        assert_eq!(app.screen.scroll_left, 0);
        assert!(preview_text.contains("Right:"));
        assert!(!preview_text.contains('┌'));
    }

    #[test]
    fn preview_exit_restores_selection_cursor_viewport_and_source_metadata() {
        let mut app = super::super::App::new(None).unwrap();
        app.file.path = Some(PathBuf::from("README"));
        app.file.text_format = crate::file::text_format::TextFormat {
            utf8_bom: true,
            line_ending: crate::file::text_format::LineEnding::Crlf,
        };
        app.buffer = Box::new(crate::buffer::PieceTable::from_text(
            "first line\nsecond line is deliberately long\nthird\nfourth\nfifth\nsixth",
        ));
        app.buffer.insert_char('!');
        app.file.dirty = true;
        app.screen.width = 12;
        app.screen.height = 4;
        let mut out = Vec::new();
        super::super::selection::move_to(&mut app, &mut out, Cursor { row: 1, col: 8 }, true)
            .unwrap();
        app.screen.scroll_top = 3;
        app.screen.scroll_left = 5;
        app.screen.wrap_col = 2;

        let source = app.buffer.to_string();
        let cursor = app.buffer.cursor();
        let selection = app.selection.active();
        let history = app.buffer.edit_history_position();
        let path = app.file.path.clone();
        let format = app.file.text_format;
        let viewport = (
            app.screen.scroll_top,
            app.screen.scroll_left,
            app.screen.wrap_col,
        );

        press(&mut app, &mut out, KeyCode::F(6));
        assert!(is_preview(&app));
        assert_eq!(app.selection.active(), selection);
        press(&mut app, &mut out, KeyCode::Esc);

        assert!(!is_preview(&app));
        assert_eq!(app.buffer.to_string(), source);
        assert_eq!(app.buffer.cursor(), cursor);
        assert_eq!(app.selection.active(), selection);
        assert_eq!(app.buffer.edit_history_position(), history);
        assert_eq!(app.file.path, path);
        assert_eq!(app.file.text_format, format);
        assert!(app.file.dirty);
        assert_eq!(
            (
                app.screen.scroll_top,
                app.screen.scroll_left,
                app.screen.wrap_col,
            ),
            viewport
        );
    }

    #[test]
    fn preview_scroll_maps_back_to_the_source_without_drifting_an_untouched_view() {
        let mut app = super::super::App::new(None).unwrap();
        let source = (0..60)
            .map(|row| format!("## Row {row:02}\n\n"))
            .collect::<String>();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text(&source));
        app.screen.width = 60;
        app.screen.height = 8;
        let visible_height = app.screen.visible_height();
        let source_maximum = app.buffer.line_count().saturating_sub(visible_height);
        let source_middle = source_maximum / 2;
        app.screen.scroll_top = source_middle;
        let mut out = Vec::new();

        press(&mut app, &mut out, KeyCode::F(6));
        let initial_preview_top = app.screen.scroll_top;
        assert!(initial_preview_top > 0);
        press(&mut app, &mut out, KeyCode::F(6));
        assert_eq!(app.screen.scroll_top, source_middle);

        press(&mut app, &mut out, KeyCode::F(6));
        let preview_maximum = display_buffer(&app)
            .line_count()
            .saturating_sub(visible_height);
        app.screen.scroll_top = preview_maximum;
        press(&mut app, &mut out, KeyCode::F(6));

        assert_eq!(app.screen.scroll_top, source_maximum);
        assert_eq!(app.buffer.to_string(), source);
    }

    #[test]
    fn viewport_mapping_is_monotonic_and_pins_both_ends() {
        assert_eq!(map_viewport_top(0, 100, 200, 10), 0);
        assert_eq!(map_viewport_top(90, 100, 200, 10), 190);
        assert_eq!(map_viewport_top(50, 5, 200, 10), 0);
        assert_eq!(map_viewport_top(50, 100, 5, 10), 0);
        assert_eq!(map_viewport_top(50, 100, 200, 0), 0);

        let mapped = (0..=90)
            .map(|position| map_viewport_top(position, 100, 200, 10))
            .collect::<Vec<_>>();
        assert!(mapped.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    #[test]
    fn active_preview_reflows_when_the_terminal_becomes_narrower() {
        let mut app = super::super::App::new(None).unwrap();
        let source = "A long paragraph with Unicode 猫🐾 and a URL https://example.com/a/long/path that must reflow after resize.";
        app.buffer = Box::new(crate::buffer::PieceTable::from_text(source));
        app.screen.width = 60;
        let mut out = Vec::new();
        press(&mut app, &mut out, KeyCode::F(6));

        app.handle_resize(16, 10, &mut out).unwrap();

        let preview = app.view.preview.as_ref().unwrap().buffer.to_string();
        assert!(preview
            .lines()
            .all(|line| crate::editor::text_layout::cell_width_from(line, 0) <= 16));
        assert_eq!(app.buffer.to_string(), source);
    }
}
