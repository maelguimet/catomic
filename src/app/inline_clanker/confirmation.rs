//! Purpose: present and operate the explicit inline-clanker send confirmation.
//! Owns: auditable confirmation text, read-only navigation, and source viewport restoration.
//! Must not: construct clients, start requests without Enter, mutate source text, or save files.
//! Invariants: destination, scope, request sizes, sensitivity, and cleanup are visible before send.
//! Phase: issue #65 one-key inline clanker workflow.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent};

use crate::buffer::{Buffer, PieceTable};
use crate::llm::context::Sensitivity;
use crate::llm::inline::InlineScope;

use super::{Phase, PreparedWorkflow, SendConfirmation};

pub(super) fn show(
    app: &mut super::super::App,
    out: &mut dyn Write,
    prepared: PreparedWorkflow,
) -> io::Result<()> {
    let buffer = PieceTable::from_text(&document(&prepared));
    let confirmation = SendConfirmation {
        prepared,
        buffer,
        source_scroll_top: app.screen.scroll_top,
        source_scroll_left: app.screen.scroll_left,
        source_wrap_col: app.screen.wrap_col,
    };
    app.selection.clear();
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    app.screen.wrap_col = 0;
    app.message = Some(
        "Inline clanker send confirmation (read-only): review details above; Enter sends; Esc cancels."
            .to_string(),
    );
    app.inline_clanker.phase = Some(Phase::Confirm(Box::new(confirmation)));
    app.render(out)
}

pub(super) fn handle_key(
    app: &mut super::super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<()> {
    match key.code {
        KeyCode::Enter => super::request::confirm(app, out),
        KeyCode::Esc => cancel(app, out),
        KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
            move_cursor(app, key.code);
            reveal(app);
            app.render(out)
        }
        KeyCode::PageUp | KeyCode::PageDown => {
            let direction = if key.code == KeyCode::PageUp {
                KeyCode::Up
            } else {
                KeyCode::Down
            };
            for _ in 0..app.screen.visible_height().max(1) {
                move_cursor(app, direction);
            }
            reveal(app);
            app.render(out)
        }
        _ => {
            app.message = Some(
                "Inline clanker confirmation is read-only. Enter sends; Esc cancels.".to_string(),
            );
            app.render(out)
        }
    }
}

pub(super) fn restore_view(app: &mut super::super::App, confirmation: &SendConfirmation) {
    app.screen.scroll_top = confirmation.source_scroll_top;
    app.screen.scroll_left = confirmation.source_scroll_left;
    app.screen.wrap_col = confirmation.source_wrap_col;
}

fn cancel(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    if let Some(Phase::Confirm(confirmation)) = app.inline_clanker.phase.take() {
        restore_view(app, &confirmation);
    }
    app.message = Some(
        "Inline clanker cancelled before sending; no client or network call was started."
            .to_string(),
    );
    app.render(out)
}

fn move_cursor(app: &mut super::super::App, code: KeyCode) {
    let Some(Phase::Confirm(confirmation)) = app.inline_clanker.phase.as_mut() else {
        return;
    };
    match code {
        KeyCode::Left => confirmation.buffer.move_left(),
        KeyCode::Right => confirmation.buffer.move_right(),
        KeyCode::Up => confirmation.buffer.move_up(),
        KeyCode::Down => confirmation.buffer.move_down(),
        _ => {}
    }
}

fn reveal(app: &mut super::super::App) {
    let Some(Phase::Confirm(confirmation)) = app.inline_clanker.phase.as_ref() else {
        return;
    };
    let cursor = confirmation.buffer.cursor();
    app.screen.reveal_row(cursor.row);
    app.screen
        .reveal_col_with_width(cursor.col, super::super::view::content_width(app));
}

fn document(prepared: &PreparedWorkflow) -> String {
    let requests = prepared
        .draft
        .requests
        .iter()
        .enumerate()
        .map(|(index, request)| {
            format!(
                "Request {}: {} lines / {} bytes",
                index + 1,
                request.line_count,
                request.byte_count
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Inline clanker send confirmation\n\nPreset: {}\nModel: {}\nAdapter: {}\nDestination: {}\nInstruction source: line {}\nInstruction:\n{}\n\nMode: {}\nScope: {}\n{}\nSensitive content: {}\nCleanup after final accepted proposal: {}\n\nNo client, process, or network request starts until Enter.\nEnter sends; Esc cancels.\n",
        prepared.preset.name,
        prepared.preset.model,
        prepared.preset.adapter_label(),
        prepared.destination,
        prepared.draft.instruction.display_line,
        prepared.draft.instruction.text,
        prepared.draft.block_mode_label(&prepared.inline),
        scope_summary(prepared),
        requests,
        sensitivity_summary(prepared),
        if prepared.inline.remove_instruction_after_apply {
            "enabled"
        } else {
            "disabled"
        },
    )
}

fn scope_summary(prepared: &PreparedWorkflow) -> String {
    match prepared.draft.scope {
        InlineScope::Selection => {
            let range = &prepared.draft.targets[0].range;
            format!(
                "selection lines {}-{} ({} bytes)",
                range.first_line + 1,
                range.last_line + 1,
                range.original.len()
            )
        }
        InlineScope::Blocks => {
            let ranges = prepared
                .draft
                .targets
                .iter()
                .map(|target| {
                    format!(
                        "{}:{}-{}({}B)",
                        target.id,
                        target.range.first_line + 1,
                        target.range.last_line + 1,
                        target.range.original.len()
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "{} context block(s) [{ranges}]",
                prepared.draft.targets.len()
            )
        }
        InlineScope::FullFile => format!(
            "bounded full file {} lines/{} bytes",
            prepared.draft.full_file_lines, prepared.draft.full_file_bytes
        ),
    }
}

pub(super) fn sensitivity_summary(prepared: &PreparedWorkflow) -> String {
    if prepared.draft.sensitivity.is_empty() {
        return "No obvious sensitive content detected.".to_string();
    }
    let labels = prepared
        .draft
        .sensitivity
        .iter()
        .map(|warning| match warning {
            Sensitivity::Dotfile => "dotfile".to_string(),
            Sensitivity::SecretLikeLine { line } => format!("secret-like line {}", line + 1),
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("Obvious sensitive content detected: {labels}.")
}
