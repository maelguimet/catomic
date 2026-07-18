//! Purpose: display and operate the read-only inline-clanker proposal surface.
//! Owns: preview navigation, final drift validation, apply, queue advance, and rejection.
//! Must not: parse model output, construct clients, save files, or widen captured ranges.
//! Invariants: Enter is the only apply path; every accepted batch is one buffer transaction.
//! Phase: issue #65 one-key inline clanker workflow.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent};

use crate::buffer::{Buffer, PieceTable};

use super::{ChangeSet, PendingEdit, Phase, PreparedWorkflow, ProposalPreview};

pub(super) fn open(
    app: &mut super::super::App,
    out: &mut dyn Write,
    prepared: PreparedWorkflow,
    edits: Vec<PendingEdit>,
    applied_changes: ChangeSet,
    preview_text: String,
    preview_changes: ChangeSet,
) -> io::Result<()> {
    super::super::view::cancel_preview(app);
    super::super::lint::close_view(app);
    super::super::project_files::close_view(app);
    super::super::llm_preview::close(app);
    let source_scroll_top = app.screen.scroll_top;
    let source_scroll_left = app.screen.scroll_left;
    let source_wrap_col = app.screen.wrap_col;
    app.inline_clanker.phase = Some(Phase::Preview(Box::new(ProposalPreview {
        source_revision: prepared.expected_revision,
        source_path: prepared.file_path.clone(),
        prepared,
        edits,
        buffer: PieceTable::from_text(&preview_text),
        preview_changes,
        applied_changes,
        source_scroll_top,
        source_scroll_left,
        source_wrap_col,
    })));
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    app.screen.wrap_col = 0;
    app.selection.clear();
    app.message = Some(
        "Inline clanker proposal (read-only): touched ranges are red/underlined. Enter applies the complete diff; Esc rejects."
            .to_string(),
    );
    app.render(out)
}

pub(super) fn handle_key(
    app: &mut super::super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<()> {
    match key.code {
        KeyCode::Enter => apply(app, out),
        KeyCode::Esc => cancel(app, out),
        KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
            move_cursor(app, key.code);
            reveal(app);
            app.render(out)
        }
        KeyCode::PageUp | KeyCode::PageDown => {
            for _ in 0..app.screen.visible_height().max(1) {
                move_cursor(
                    app,
                    if key.code == KeyCode::PageUp {
                        KeyCode::Up
                    } else {
                        KeyCode::Down
                    },
                );
            }
            reveal(app);
            app.render(out)
        }
        _ => {
            app.message = Some(
                "Inline clanker proposal is read-only. Enter applies; Esc rejects.".to_string(),
            );
            app.render(out)
        }
    }
}

fn apply(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    let Some(Phase::Preview(preview)) = app.inline_clanker.phase.take() else {
        return Ok(());
    };
    app.screen.scroll_top = preview.source_scroll_top;
    app.screen.scroll_left = preview.source_scroll_left;
    app.screen.wrap_col = preview.source_wrap_col;
    if proposal_drifted(app, &preview) {
        app.message = Some(
            "Inline clanker apply refused: source, instruction, delimiter, or target drifted."
                .to_string(),
        );
        return app.render(out);
    }
    let before = app.buffer.edit_history_position();
    let text_edits: Vec<_> = preview
        .edits
        .iter()
        .map(|pending| pending.edit.clone())
        .collect();
    let mut cumulative = if preview.prepared.applied_count == 0 {
        ChangeSet::default()
    } else {
        app.clanker_changes.changes_at(before)
    };
    super::changes::shift_set(&mut cumulative, &text_edits);
    cumulative
        .ranges
        .extend(preview.applied_changes.ranges.iter().copied());
    cumulative
        .gutter_lines
        .extend(preview.applied_changes.gutter_lines.iter().copied());
    cumulative.gutter_lines.sort_unstable();
    cumulative.gutter_lines.dedup();
    if app.buffer.replace_text_edits(&text_edits)? == 0 {
        app.message = Some("Inline clanker proposal makes no applicable change.".to_string());
        return app.render(out);
    }
    let after = app.buffer.edit_history_position();
    app.clanker_changes.record(
        preview.prepared.applied_count == 0,
        before,
        after,
        cumulative,
    );
    continue_or_finish(app, out, *preview, after)
}

fn proposal_drifted(app: &super::super::App, preview: &ProposalPreview) -> bool {
    app.file.path != preview.source_path
        || app.buffer.edit_history_position() != preview.source_revision
        || super::request::validate_identity(app, &preview.prepared).is_err()
        || preview.edits.iter().any(|pending| {
            app.buffer
                .text_range(pending.edit.start, pending.edit.end)
                .map_or(true, |text| text != pending.original)
        })
}

fn continue_or_finish(
    app: &mut super::super::App,
    out: &mut dyn Write,
    mut preview: ProposalPreview,
    after: u64,
) -> io::Result<()> {
    let has_more = preview.prepared.request_index + 1 < preview.prepared.draft.requests.len();
    if has_more {
        update_queued_draft(&mut preview);
        preview.prepared.expected_revision = after;
        preview.prepared.request_index += 1;
        preview.prepared.applied_count += 1;
        super::super::input::finish_content_edit_with_message(
            app,
            out,
            Some("Inline clanker block applied; starting the next serial block.".to_string()),
        )?;
        return super::request::start(app, out, preview.prepared);
    }
    let message = Some(
        "Inline clanker proposal applied without saving. Red/underlined text and gutter marks are locally applied model output, not a selection; Ctrl+Z undoes it."
            .to_string(),
    );
    super::super::input::finish_content_edit_with_message(app, out, message)
}

fn update_queued_draft(preview: &mut ProposalPreview) {
    for pending in &preview.edits {
        if let Some(id) = pending.target_id {
            super::changes::update_draft_after_edit(&mut preview.prepared.draft, id, &pending.edit);
        }
    }
}

fn cancel(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    let Some(Phase::Preview(preview)) = app.inline_clanker.phase.take() else {
        return Ok(());
    };
    app.screen.scroll_top = preview.source_scroll_top;
    app.screen.scroll_left = preview.source_scroll_left;
    app.screen.wrap_col = preview.source_wrap_col;
    app.message = Some(
        "Inline clanker proposal rejected; remaining queue cleared, instruction kept, and no previewed changes applied."
            .to_string(),
    );
    app.reveal_cursor();
    app.render(out)
}

fn move_cursor(app: &mut super::super::App, code: KeyCode) {
    let Some(Phase::Preview(preview)) = app.inline_clanker.phase.as_mut() else {
        return;
    };
    match code {
        KeyCode::Left => preview.buffer.move_left(),
        KeyCode::Right => preview.buffer.move_right(),
        KeyCode::Up => preview.buffer.move_up(),
        KeyCode::Down => preview.buffer.move_down(),
        _ => {}
    }
}

fn reveal(app: &mut super::super::App) {
    let Some(Phase::Preview(preview)) = app.inline_clanker.phase.as_ref() else {
        return;
    };
    let cursor = preview.buffer.cursor();
    app.screen.reveal_row(cursor.row);
    app.screen
        .reveal_col_with_width(cursor.col, super::super::view::content_width(app));
}
