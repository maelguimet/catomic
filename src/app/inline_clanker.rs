//! Purpose: own the explicit F3 inline-clanker workflow without changing legacy model commands.
//! Owns: warning/confirmation/request/preview phases, serial queue lifetime, and change metadata.
//! Must not: start network work before confirmation, save files, or send out-of-scope text.
//! Invariants: at most one request runs; every apply revalidates path, revision, guards, and ranges.

use std::io::{self, Write};
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{PieceTable, TextEdit};
use crate::config::llm::{BackendPreset, InlineSettings};
use crate::llm::inline::InlineDraft;
use crate::llm::task::LlmTask;

mod changes;
mod confirmation;
mod prepare;
mod preview;
mod preview_ui;
mod request;

pub(crate) use changes::{ChangeHistory, ChangedRange, VisibleChanges};

#[derive(Default)]
pub(crate) struct InlineClankerState {
    phase: Option<Phase>,
}

enum Phase {
    Warning(PreparedWorkflow),
    Confirm(Box<SendConfirmation>),
    Running(RunningRequest),
    Preview(Box<ProposalPreview>),
}

struct SendConfirmation {
    prepared: PreparedWorkflow,
    buffer: PieceTable,
    source_scroll_top: usize,
    source_scroll_left: usize,
    source_wrap_col: usize,
}

struct PreparedWorkflow {
    draft: InlineDraft,
    preset: BackendPreset,
    inline: InlineSettings,
    destination: String,
    path: String,
    file_path: Option<PathBuf>,
    expected_revision: u64,
    request_index: usize,
    applied_count: usize,
    had_failure: bool,
}

struct RunningRequest {
    prepared: PreparedWorkflow,
    task: LlmTask,
}

#[derive(Clone)]
struct PendingEdit {
    target_id: Option<usize>,
    edit: TextEdit,
    original: String,
    label: String,
}

struct ProposalPreview {
    prepared: PreparedWorkflow,
    edits: Vec<PendingEdit>,
    source_revision: u64,
    source_path: Option<PathBuf>,
    buffer: PieceTable,
    preview_changes: ChangeSet,
    applied_changes: ChangeSet,
    source_scroll_top: usize,
    source_scroll_left: usize,
    source_wrap_col: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ChangeSet {
    ranges: Vec<ChangedRange>,
    gutter_lines: Vec<usize>,
}

pub(crate) fn begin(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    prepare::begin(app, out)
}

pub(crate) fn answer_warning(
    app: &mut super::App,
    out: &mut dyn Write,
    answer: &str,
) -> io::Result<bool> {
    prepare::answer_warning(app, out, answer)
}

pub(crate) use prepare::{cancel_warning, warning_prompt_message};

pub(crate) fn poll(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    request::poll(app, out)
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    let Some(phase) = app.inline_clanker.phase.as_ref() else {
        return Ok(false);
    };
    if is_quit(key) {
        return Ok(false);
    }
    match phase {
        Phase::Warning(_) => Ok(false),
        Phase::Confirm(_) => {
            confirmation::handle_key(app, out, key)?;
            Ok(true)
        }
        Phase::Running(_) => {
            if key.code == KeyCode::Esc {
                cancel_running(app, out)?;
            } else {
                app.message_info(
                    "Inline clanker request is running. Esc cancels; other input is paused.",
                );
                app.render(out)?;
            }
            Ok(true)
        }
        Phase::Preview(_) => {
            preview_ui::handle_key(app, out, key)?;
            Ok(true)
        }
    }
}

pub(crate) fn handle_paste(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    match app.inline_clanker.phase {
        Some(Phase::Warning(_)) | Some(Phase::Confirm(_)) => {
            app.message_info("Inline clanker confirmation is read-only. Enter sends; Esc cancels.");
            app.render(out)?;
            Ok(true)
        }
        Some(Phase::Running(_)) => {
            app.message_info(
                "Inline clanker request is running. Esc cancels; pasted input is paused.",
            );
            app.render(out)?;
            Ok(true)
        }
        Some(Phase::Preview(_)) => {
            app.message_info("Inline clanker proposal is read-only. Enter applies; Esc rejects.");
            app.render(out)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub(crate) fn display_buffer(app: &super::App) -> Option<&dyn crate::buffer::Buffer> {
    match app.inline_clanker.phase.as_ref() {
        Some(Phase::Confirm(confirmation)) => Some(&confirmation.buffer),
        Some(Phase::Preview(preview)) => Some(&preview.buffer),
        _ => None,
    }
}

pub(crate) fn preview_changes(app: &super::App) -> Option<VisibleChanges<'_>> {
    match app.inline_clanker.phase.as_ref() {
        Some(Phase::Preview(preview)) => Some(VisibleChanges::from_set(&preview.preview_changes)),
        _ => None,
    }
}

pub(crate) fn source_changes(app: &super::App) -> Option<VisibleChanges<'_>> {
    app.clanker_changes
        .visible(app.buffer.edit_history_position())
}

pub(crate) fn is_previewing(app: &super::App) -> bool {
    matches!(
        app.inline_clanker.phase,
        Some(Phase::Confirm(_) | Phase::Preview(_))
    )
}

pub(crate) fn is_busy(app: &super::App) -> bool {
    app.inline_clanker.phase.is_some()
}

pub(crate) fn clear_changes(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    app.clanker_changes.clear();
    app.message = None;
    app.render(out)
}

pub(crate) fn cancel_all(app: &mut super::App) -> bool {
    let Some(phase) = app.inline_clanker.phase.take() else {
        return false;
    };
    match phase {
        Phase::Confirm(confirmation) => confirmation::restore_view(app, &confirmation),
        Phase::Preview(preview) => {
            app.screen.scroll_top = preview.source_scroll_top;
            app.screen.scroll_left = preview.source_scroll_left;
            app.screen.wrap_col = preview.source_wrap_col;
        }
        Phase::Warning(_) | Phase::Running(_) => {}
    }
    true
}

fn cancel_running(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    app.inline_clanker.phase = None;
    app.message = None;
    app.render(out)
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests;
