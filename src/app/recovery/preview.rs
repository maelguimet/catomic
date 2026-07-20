//! Purpose: present `.catnap` content read-only and apply it only after Enter.
//! Owns: preview construction, navigation, drift checks, one edit, cancel, and display buffer.
//! Must not: write source files, schedule autosave, load config, remove sidecars, or network.
//! Invariants: source stays unchanged until Enter; source or retained-candidate drift refuses apply.
//! Phase: 8 recovery preview.

use std::io::{self, Write};
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Buffer, Cursor, PieceTable};
use crate::file::io::FileSnapshot;
use crate::file::recovery::RecoveryCandidate;

pub(super) struct RecoveryPreview {
    buffer: PieceTable,
    candidate: RecoveryCandidate,
    source_path: Option<PathBuf>,
    source_history: u64,
    source_disk_snapshot: Option<FileSnapshot>,
    source_scroll_top: usize,
    source_scroll_left: usize,
}

pub(crate) fn start_preview(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    let config = app.cat_config.recovery;
    if !config.enabled {
        app.message = Some("Catnap recovery is disabled in [recovery].".to_string());
        return app.render(out);
    }
    if super::super::external_command::is_busy(app) {
        app.message = Some("Finish the active external command before recovery.".to_string());
        return app.render(out);
    }
    let Some(path) = app.file.path.clone() else {
        app.message = Some("Catnap recovery requires a named file.".to_string());
        return app.render(out);
    };
    let mut candidate = match app.recovery.offered_candidate.take() {
        Some(candidate) => candidate,
        None => match crate::file::recovery::load_candidate(&path, config.max_bytes) {
            Ok(Some(candidate)) => candidate,
            Ok(None) => {
                app.message = Some("No newer catnap recovery is available.".to_string());
                return app.render(out);
            }
            Err(error) => return preview_error(app, out, error),
        },
    };
    match candidate.is_current(&path) {
        Ok(true) => {}
        Ok(false) => {
            app.message = Some("No newer catnap recovery is available.".to_string());
            return app.render(out);
        }
        Err(error) => return preview_error(app, out, error),
    }
    open(app, out, candidate)
}

fn preview_error(
    app: &mut super::super::App,
    out: &mut dyn Write,
    error: io::Error,
) -> io::Result<()> {
    app.message = Some(format!("Cannot open catnap recovery: {error}"));
    app.render(out)
}

fn open(
    app: &mut super::super::App,
    out: &mut dyn Write,
    candidate: RecoveryCandidate,
) -> io::Result<()> {
    super::super::view::cancel_preview(app);
    super::super::llm_preview::close(app);
    super::super::llm_answer::close(app);
    super::super::lint::close_view(app);
    super::super::project_files::close_view(app);
    app.recovery.preview = Some(RecoveryPreview {
        buffer: PieceTable::from_text(candidate.text()),
        candidate,
        source_path: app.file.path.clone(),
        source_history: app.buffer.edit_history_position(),
        source_disk_snapshot: app.file.disk_snapshot.clone(),
        source_scroll_top: app.screen.scroll_top,
        source_scroll_left: app.screen.scroll_left,
    });
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    app.selection.clear();
    preview_message(app);
    app.render(out)
}

pub(crate) fn handle_key(
    app: &mut super::super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if !is_viewing(app) || is_quit(key) {
        return Ok(false);
    }
    match key.code {
        KeyCode::Enter => apply(app, out)?,
        KeyCode::Esc => cancel(app, out)?,
        KeyCode::Left => move_cursor(app, |buffer| buffer.move_left()),
        KeyCode::Right => move_cursor(app, |buffer| buffer.move_right()),
        KeyCode::Up => move_cursor(app, |buffer| buffer.move_up()),
        KeyCode::Down => move_cursor(app, |buffer| buffer.move_down()),
        _ => preview_message(app),
    }
    if is_viewing(app) {
        reveal_cursor(app);
        app.render(out)?;
    }
    Ok(true)
}

pub(crate) fn handle_paste(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<bool> {
    if !is_viewing(app) {
        return Ok(false);
    }
    preview_message(app);
    app.render(out)?;
    Ok(true)
}

fn apply(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    let mut preview = app.recovery.preview.take().expect("recovery preview");
    restore_scroll(app, &preview);
    if app.file.path != preview.source_path
        || app.buffer.edit_history_position() != preview.source_history
        || app.file.disk_snapshot != preview.source_disk_snapshot
    {
        app.message = Some("Source changed during recovery preview; nothing applied.".to_string());
        return app.render(out);
    }
    let candidate_is_current = preview
        .source_path
        .as_deref()
        .map(|path| preview.candidate.is_current(path).unwrap_or(false))
        .unwrap_or(false);
    if !candidate_is_current {
        app.message = Some("Catnap changed during recovery preview; nothing applied.".to_string());
        return app.render(out);
    }
    if !replace_buffer(&mut *app.buffer, preview.candidate.text())? {
        app.message = Some("Recovery already matches the current buffer.".to_string());
        return app.render(out);
    }
    super::super::input::finish_content_edit_with_message(
        app,
        out,
        Some("Catnap recovered; Ctrl+Z undoes it. Save explicitly when ready.".to_string()),
    )
}

fn replace_buffer(buffer: &mut dyn Buffer, text: &str) -> io::Result<bool> {
    let row = buffer.line_count().saturating_sub(1);
    let end = Cursor {
        row,
        col: buffer.line_char_count(row).unwrap_or(0),
    };
    buffer.replace_range(Cursor::default(), end, text)
}

fn cancel(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    close(app);
    app.message = None;
    app.reveal_cursor();
    app.render(out)
}

pub(crate) fn close(app: &mut super::super::App) -> bool {
    if let Some(preview) = app.recovery.preview.take() {
        restore_scroll(app, &preview);
        true
    } else {
        false
    }
}

fn restore_scroll(app: &mut super::super::App, preview: &RecoveryPreview) {
    app.screen.scroll_top = preview.source_scroll_top;
    app.screen.scroll_left = preview.source_scroll_left;
}

fn move_cursor(app: &mut super::super::App, movement: impl FnOnce(&mut PieceTable)) {
    movement(&mut app.recovery.preview.as_mut().expect("preview").buffer);
}

fn reveal_cursor(app: &mut super::super::App) {
    let cursor = app
        .recovery
        .preview
        .as_ref()
        .expect("preview")
        .buffer
        .cursor();
    app.screen.reveal_row(cursor.row);
    app.screen
        .reveal_col_with_width(cursor.col, super::super::view::content_width(app));
}

fn preview_message(app: &mut super::super::App) {
    app.message = Some("Catnap preview (read-only). Enter recovers; Esc cancels.".to_string());
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}

pub(crate) fn is_viewing(app: &super::super::App) -> bool {
    app.recovery.preview.is_some()
}

pub(crate) fn display_buffer(app: &super::super::App) -> Option<&dyn Buffer> {
    app.recovery
        .preview
        .as_ref()
        .map(|preview| &preview.buffer as &dyn Buffer)
}
