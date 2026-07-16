//! Purpose: preview external output and apply it only after explicit confirmation.
//! Owns: read-only result view, navigation, stale-source checks, and one edit transaction.
//! Must not: spawn processes, load config, write files, or apply failed/truncated output.
//! Invariants: Enter alone applies successful complete output; source/path drift refuses it.
//! Phase: 7 external command preview and undo safety.

use std::io::{self, Write};
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Buffer, Cursor, PieceTable};

use super::{ApplyTarget, RunningCommand};

pub(super) struct CommandPreview {
    name: String,
    proposed_text: String,
    pub(super) target: Option<ApplyTarget>,
    succeeded: bool,
    source_snapshot: Option<String>,
    source_path: Option<PathBuf>,
    buffer: PieceTable,
    source_scroll_top: usize,
    source_scroll_left: usize,
}

pub(super) fn open(
    app: &mut super::super::App,
    out: &mut dyn Write,
    mut running: RunningCommand,
    stdout: String,
    stderr: String,
    code: Option<i32>,
    truncated: bool,
) -> io::Result<()> {
    let succeeded = code == Some(0) && !truncated;
    if !succeeded {
        running.target = None;
    }
    let text = result_text(&stdout, &stderr, code, truncated);
    super::super::view::cancel_preview(app);
    super::super::llm_preview::close(app);
    super::super::llm_answer::close(app);
    super::super::lint::close_view(app);
    super::super::project_files::close_view(app);
    app.external_command.preview = Some(CommandPreview {
        name: running.name,
        proposed_text: stdout,
        target: running.target,
        succeeded,
        source_snapshot: running.source_snapshot,
        source_path: running.source_path,
        buffer: PieceTable::from_text(&text),
        source_scroll_top: app.screen.scroll_top,
        source_scroll_left: app.screen.scroll_left,
    });
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    app.selection.clear();
    read_only_message(app);
    app.render(out)
}

pub(super) fn handle_key(
    app: &mut super::super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if !is_viewing(app) || is_quit(key) {
        return Ok(false);
    }
    match key.code {
        KeyCode::Enter => apply_or_close(app, out)?,
        KeyCode::Esc => cancel(app, out)?,
        KeyCode::Left => move_cursor(app, Move::Left),
        KeyCode::Right => move_cursor(app, Move::Right),
        KeyCode::Up => move_cursor(app, Move::Up),
        KeyCode::Down => move_cursor(app, Move::Down),
        KeyCode::PageUp => move_page(app, false),
        KeyCode::PageDown => move_page(app, true),
        KeyCode::Home => set_line_edge(app, false),
        KeyCode::End => set_line_edge(app, true),
        _ => read_only_message(app),
    }
    if is_viewing(app) {
        reveal_cursor(app);
        app.render(out)?;
    }
    Ok(true)
}

pub(super) fn handle_paste(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<bool> {
    if !is_viewing(app) {
        return Ok(false);
    }
    read_only_message(app);
    app.render(out)?;
    Ok(true)
}

pub(super) fn is_viewing(app: &super::super::App) -> bool {
    app.external_command.preview.is_some()
}

pub(super) fn display_buffer(app: &super::super::App) -> Option<&dyn Buffer> {
    app.external_command
        .preview
        .as_ref()
        .map(|preview| &preview.buffer as &dyn Buffer)
}

pub(super) fn close(app: &mut super::super::App) -> bool {
    if let Some(preview) = app.external_command.preview.take() {
        restore_scroll(app, &preview);
        true
    } else {
        false
    }
}

fn result_text(stdout: &str, stderr: &str, code: Option<i32>, truncated: bool) -> String {
    let mut text = if stdout.is_empty() {
        "[no stdout]\n".to_string()
    } else {
        stdout.to_string()
    };
    if !stderr.is_empty() {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("\n[stderr]\n");
        text.push_str(stderr);
    }
    if code != Some(0) || truncated {
        text.push_str(&format!(
            "\n\n[catomic: exit={code:?}{}]\n",
            if truncated { ", output truncated" } else { "" }
        ));
    }
    text
}

fn apply_or_close(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    let preview = app.external_command.preview.take().expect("preview active");
    restore_scroll(app, &preview);
    let Some(target) = preview.target else {
        app.message = Some(format!("Closed command {} output.", preview.name));
        super::super::hooks::finish_command(app, preview.succeeded);
        app.reveal_cursor();
        return app.render(out);
    };
    if app.file.path != preview.source_path
        || preview.source_snapshot.as_deref() != Some(&app.buffer.to_string())
    {
        app.message =
            Some("Source changed since command start; output was not applied.".to_string());
        super::super::hooks::finish_command(app, false);
        app.reveal_cursor();
        return app.render(out);
    }
    let changed = match target {
        ApplyTarget::Insert(at) => app.buffer.replace_range(at, at, &preview.proposed_text)?,
        ApplyTarget::ReplaceSelection(start, end) => {
            app.buffer
                .replace_range(start, end, &preview.proposed_text)?
        }
        ApplyTarget::ReplaceBuffer => replace_buffer(&mut *app.buffer, &preview.proposed_text)?,
    };
    if !changed {
        app.message = Some("Command output made no change.".to_string());
        super::super::hooks::finish_command(app, true);
        app.reveal_cursor();
        return app.render(out);
    }
    super::super::hooks::finish_command(app, true);
    super::super::input::finish_content_edit_with_message(
        app,
        out,
        Some(format!(
            "Command {} applied; Ctrl+Z undoes it.",
            preview.name
        )),
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
    let name = app
        .external_command
        .preview
        .as_ref()
        .map(|preview| preview.name.clone())
        .unwrap_or_default();
    close(app);
    app.message = Some(format!(
        "Command {name} output cancelled; no changes applied."
    ));
    super::super::hooks::finish_command(app, false);
    app.reveal_cursor();
    app.render(out)
}

fn restore_scroll(app: &mut super::super::App, preview: &CommandPreview) {
    app.screen.scroll_top = preview.source_scroll_top;
    app.screen.scroll_left = preview.source_scroll_left;
}

enum Move {
    Left,
    Right,
    Up,
    Down,
}

fn move_cursor(app: &mut super::super::App, movement: Move) {
    let buffer = &mut app
        .external_command
        .preview
        .as_mut()
        .expect("preview")
        .buffer;
    match movement {
        Move::Left => buffer.move_left(),
        Move::Right => buffer.move_right(),
        Move::Up => buffer.move_up(),
        Move::Down => buffer.move_down(),
    }
}

fn move_page(app: &mut super::super::App, forward: bool) {
    for _ in 0..app.screen.visible_height().max(1) {
        move_cursor(app, if forward { Move::Down } else { Move::Up });
    }
}

fn set_line_edge(app: &mut super::super::App, end: bool) {
    let buffer = &mut app
        .external_command
        .preview
        .as_mut()
        .expect("preview")
        .buffer;
    let row = buffer.cursor().row;
    let col = if end {
        buffer.line_char_count(row).unwrap_or(0)
    } else {
        0
    };
    buffer.set_cursor(Cursor { row, col });
}

fn reveal_cursor(app: &mut super::super::App) {
    let cursor = app
        .external_command
        .preview
        .as_ref()
        .expect("preview")
        .buffer
        .cursor();
    app.screen.reveal_row(cursor.row);
    app.screen
        .reveal_col_with_width(cursor.col, super::super::view::content_width(app));
}

fn read_only_message(app: &mut super::super::App) {
    let preview = app.external_command.preview.as_ref().expect("preview");
    app.message = Some(if preview.target.is_some() {
        format!(
            "Command {} output (read-only). Enter applies; Esc cancels.",
            preview.name
        )
    } else {
        format!(
            "Command {} output (read-only). Enter or Esc closes.",
            preview.name
        )
    });
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}
