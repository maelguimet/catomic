//! Purpose: this file must preview and explicitly confirm validated LLM edit proposals.
//! Owns: patch/marked-region preview state, stale-source checks, and confirmed apply.
//! Must not: construct clients, call endpoints, read repos, write files, or auto-apply.
//! Invariants: Enter is the only apply action; apply is one undoable buffer transaction.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Buffer, Cursor, PieceTable};
use crate::config::actions::Action;
use crate::llm::broker::ContextBroker;

mod confirm;
mod proposal;
mod repo;

use proposal::Proposal;
pub(crate) use proposal::RegionTarget;
pub(crate) use repo::show_repo_patch;

pub(crate) struct PatchPreview {
    proposal: Proposal,
    proposed_text: String,
    source_snapshot: String,
    source_path: Option<std::path::PathBuf>,
    repo_guard: Option<ContextBroker>,
    buffer: PieceTable,
    source_scroll_top: usize,
    source_scroll_left: usize,
}

struct PreviewDraft<'a> {
    proposal: Proposal,
    proposed_text: String,
    source_snapshot: String,
    preview_text: &'a str,
    message: &'static str,
    repo_guard: Option<ContextBroker>,
}

#[cfg(test)]
pub(crate) fn show(app: &mut super::App, out: &mut dyn Write, patch_text: &str) -> io::Result<()> {
    if app.buffer.is_read_only() || app.buffer.page_info().is_some() {
        app.message_info("LLM patch preview requires a fully editable current buffer.");
        return app.render(out);
    }
    let source_snapshot = app.buffer.to_string();
    let (proposal, proposed_text) = match proposal::build_patch(&source_snapshot, patch_text) {
        Ok(proposal) => proposal,
        Err(message) => {
            app.message_error(message);
            return app.render(out);
        }
    };

    open(
        app,
        out,
        PreviewDraft {
            proposal,
            proposed_text,
            source_snapshot,
            preview_text: patch_text,
            message: "LLM patch preview (read-only). Enter applies; Esc cancels.",
            repo_guard: None,
        },
    )
}

pub(crate) fn show_with_region_fallback(
    app: &mut super::App,
    out: &mut dyn Write,
    output: &str,
    expected_path: Option<&str>,
    target: Option<RegionTarget>,
) -> io::Result<()> {
    let source_snapshot = app.buffer.to_string();
    let patch = match expected_path {
        Some(path) => proposal::build_patch_for_path(&source_snapshot, output, path),
        None => proposal::build_patch(&source_snapshot, output),
    };
    if let Ok((proposal, proposed_text)) = patch {
        return open(
            app,
            out,
            PreviewDraft {
                proposal,
                proposed_text,
                source_snapshot,
                preview_text: output,
                message: "LLM patch preview (read-only). Enter applies; Esc cancels.",
                repo_guard: None,
            },
        );
    }
    let Some(target) = target else {
        app.message_error("Invalid LLM patch; no marked selection fallback was available.");
        return app.render(out);
    };
    if app.buffer.text_range(target.start(), target.end())? != target.original() {
        app.message_warning("Selected text changed; LLM replacement was not previewed.");
        return app.render(out);
    }
    let (region, replacement, preview_text) = match proposal::build_region(output, target) {
        Ok(proposal) => proposal,
        Err(error) => {
            app.message_error(error);
            return app.render(out);
        }
    };
    open(
        app,
        out,
        PreviewDraft {
            proposal: region,
            proposed_text: replacement,
            source_snapshot,
            preview_text: &preview_text,
            message: "LLM marked-region preview (read-only). Enter applies; Esc cancels.",
            repo_guard: None,
        },
    )
}

fn open(app: &mut super::App, out: &mut dyn Write, draft: PreviewDraft<'_>) -> io::Result<()> {
    super::view::cancel_preview(app);
    super::lint::close_view(app);
    super::project_files::close_view(app);
    close(app);
    app.surfaces.llm_preview = Some(PatchPreview {
        proposal: draft.proposal,
        proposed_text: draft.proposed_text,
        source_snapshot: draft.source_snapshot,
        source_path: app.file.path.clone(),
        repo_guard: draft.repo_guard,
        buffer: PieceTable::from_text(draft.preview_text),
        source_scroll_top: app.screen.scroll_top,
        source_scroll_left: app.screen.scroll_left,
    });
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    app.selection.clear();
    app.message_info(draft.message.to_string());
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
        KeyCode::Enter => confirm::apply(app, out)?,
        KeyCode::Esc => cancel(app, out)?,
        KeyCode::Left => move_cursor(app, Move::Left),
        KeyCode::Right => move_cursor(app, Move::Right),
        KeyCode::Up => move_cursor(app, Move::Up),
        KeyCode::Down => move_cursor(app, Move::Down),
        KeyCode::PageUp => move_page(app, false),
        KeyCode::PageDown => move_page(app, true),
        KeyCode::Home => set_line_edge(app, false),
        KeyCode::End => set_line_edge(app, true),
        _ => app.message_info("LLM patch preview is read-only. Enter applies; Esc cancels."),
    }
    if is_viewing(app) {
        reveal_preview_cursor(app);
        app.render(out)?;
    }
    Ok(true)
}

pub(crate) fn dispatch_action(
    app: &mut super::App,
    out: &mut dyn Write,
    action: Action,
) -> io::Result<bool> {
    if !is_viewing(app) {
        return Ok(false);
    }
    match action {
        Action::PreviewAccept => confirm::apply(app, out)?,
        Action::PreviewCancel => cancel(app, out)?,
        Action::MoveLeft => move_cursor(app, Move::Left),
        Action::MoveRight => move_cursor(app, Move::Right),
        Action::MoveUp => move_cursor(app, Move::Up),
        Action::MoveDown => move_cursor(app, Move::Down),
        Action::ViewportUp => move_page(app, false),
        Action::ViewportDown => move_page(app, true),
        Action::LineStart => set_line_edge(app, false),
        Action::LineEnd => set_line_edge(app, true),
        _ => return Ok(false),
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
    app.message_info("LLM patch preview is read-only. Enter applies; Esc cancels.");
    app.render(out)?;
    Ok(true)
}

pub(crate) fn is_viewing(app: &super::App) -> bool {
    app.surfaces.llm_preview.is_some()
}

pub(crate) fn display_buffer(app: &super::App) -> Option<&dyn Buffer> {
    app.surfaces
        .llm_preview
        .as_ref()
        .map(|preview| &preview.buffer as &dyn Buffer)
}

pub(crate) fn close(app: &mut super::App) -> bool {
    if let Some(preview) = app.surfaces.llm_preview.take() {
        app.screen.scroll_top = preview.source_scroll_top;
        app.screen.scroll_left = preview.source_scroll_left;
        true
    } else {
        false
    }
}

pub(super) fn finish_repo_apply(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    confirm::finish_apply(app, out)
}

fn cancel(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    close(app);
    app.message = None;
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
    let buffer = &mut app
        .surfaces
        .llm_preview
        .as_mut()
        .expect("preview active")
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
        .llm_preview
        .as_mut()
        .expect("preview active")
        .buffer;
    let row = buffer.cursor().row;
    let col = if end {
        buffer.line_char_count(row).unwrap_or(0)
    } else {
        0
    };
    buffer.set_cursor(Cursor { row, col });
}

fn reveal_preview_cursor(app: &mut super::App) {
    let cursor = app
        .surfaces
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
