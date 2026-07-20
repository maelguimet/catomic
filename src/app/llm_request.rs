//! Purpose: this file must cage explicit current-buffer LLM invocation end to end.
//! Owns: `:meow` drafts, endpoint/context confirmation, task polling, and cancellation.
//! Must not: collect repo context, create clients before Enter, apply output, or write files.
//! Invariants: pending state has no client; source drift discards output; patches go to preview.
//! Phase: 6 (LLM, Powerful but Caged).

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::llm::BackendPreset;
use crate::llm::backend::ConfirmedBackend;
use crate::llm::context::{self, RequestDraft};
use crate::llm::task::LlmTask;

mod prompt;
mod result;

use prompt::{confirmation_message, display_path, system_prompt, user_prompt, RequestPurpose};
pub(crate) use result::poll;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CurrentLlmCommand {
    Meow,
    BigMeow,
}

pub(crate) struct PendingLlmRequest {
    draft: RequestDraft,
    preset: BackendPreset,
    source_snapshot: String,
    path: String,
    destination: String,
    file_path: Option<PathBuf>,
    replacement_target: Option<super::llm_preview::RegionTarget>,
    purpose: RequestPurpose,
}

pub(crate) struct RunningLlmRequest {
    task: LlmTask,
    preset_name: String,
    source_snapshot: String,
    path: String,
    file_path: Option<PathBuf>,
    replacement_target: Option<super::llm_preview::RegionTarget>,
    purpose: RequestPurpose,
}

pub(crate) fn begin(
    app: &mut super::App,
    out: &mut dyn Write,
    command: CurrentLlmCommand,
    instruction: &str,
) -> io::Result<()> {
    let catalog = match crate::config::llm::load() {
        Ok(catalog) => catalog,
        Err(error) => {
            app.message = Some(format!("LLM config error: {error}"));
            return app.render(out);
        }
    };
    let preset = app.model_session.effective(&catalog);
    begin_with_settings(app, out, command, instruction, preset)
}

fn begin_with_settings(
    app: &mut super::App,
    out: &mut dyn Write,
    command: CurrentLlmCommand,
    instruction: &str,
    preset: BackendPreset,
) -> io::Result<()> {
    if app.pending_llm_request.is_some()
        || app.llm_task.is_some()
        || super::inline_clanker::is_busy(app)
    {
        app.message =
            Some("An LLM request is already pending or running; Esc cancels.".to_string());
        return app.render(out);
    }
    if app.buffer.is_read_only() || app.buffer.page_info().is_some() {
        app.message = Some("LLM commands require a fully editable current buffer.".to_string());
        return app.render(out);
    }
    let source_snapshot = app.buffer.to_string();
    let draft = match collect_draft(app, command, instruction, &source_snapshot) {
        Ok(draft) => draft,
        Err(error) => {
            app.message = Some(format!("Cannot prepare LLM request: {error}"));
            return app.render(out);
        }
    };
    let file_path = app.file.path.clone();
    let path = display_path(file_path.as_deref(), &preset);
    let destination = crate::llm::backend::display_destination(&preset);
    let replacement_target = replacement_target(app, &draft);
    let purpose = prompt::purpose(&draft);
    app.message = Some(confirmation_message(&draft, &preset, &destination));
    app.pending_llm_request = Some(PendingLlmRequest {
        draft,
        preset,
        source_snapshot,
        path,
        destination,
        file_path,
        replacement_target,
        purpose,
    });
    app.render(out)
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if app.pending_llm_request.is_some() {
        if is_quit(key) {
            return Ok(false);
        }
        match key.code {
            KeyCode::Enter => confirm(app, out)?,
            KeyCode::Esc => cancel_pending(app, out)?,
            _ => {
                app.message = Some("LLM send not confirmed. Enter sends; Esc cancels.".to_string());
                app.render(out)?;
            }
        }
        return Ok(true);
    }
    if key.code == KeyCode::Esc && app.llm_task.is_some() {
        app.llm_task = None;
        app.message = None;
        app.render(out)?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) fn handle_paste(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    if app.pending_llm_request.is_none() {
        return Ok(false);
    }
    app.message = Some("LLM send not confirmed. Enter sends; Esc cancels.".to_string());
    app.render(out)?;
    Ok(true)
}

pub(crate) fn cancel_all(app: &mut super::App) -> bool {
    let pending = app.pending_llm_request.take().is_some();
    let running = app.llm_task.take().is_some();
    pending || running
}

pub(super) fn is_active(app: &super::App) -> bool {
    app.pending_llm_request.is_some() || app.llm_task.is_some()
}

fn confirm(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let pending = app.pending_llm_request.take().expect("pending request");
    if app.buffer.to_string() != pending.source_snapshot {
        app.message =
            Some("Buffer changed before confirmation; LLM request cancelled.".to_string());
        return app.render(out);
    }
    if app.file.path != pending.file_path {
        app.message = Some(
            "Active file path changed before confirmation; LLM request cancelled.".to_string(),
        );
        return app.render(out);
    }
    let backend = match ConfirmedBackend::resolve(&pending.preset) {
        Ok(backend) => backend,
        Err(error) => {
            app.model_session
                .record_failure(&pending.preset.name, error.kind);
            app.message = Some(format!("Could not prepare LLM backend: {error}"));
            return app.render(out);
        }
    };
    if backend.destination() != pending.destination {
        app.model_session.record_failure(
            &pending.preset.name,
            crate::llm::backend::BackendErrorKind::Unavailable,
        );
        app.message = Some(
            "Configured command identity changed after confirmation; request cancelled."
                .to_string(),
        );
        return app.render(out);
    }
    let user = user_prompt(&pending.draft, &pending.path);
    match LlmTask::start(backend, system_prompt(pending.purpose).to_string(), user) {
        Ok(task) => {
            app.message = Some(format!(
                "Sending {} lines/{} bytes with preset {} model {} to {}... Esc cancels.",
                pending.draft.context.line_count,
                pending.draft.context.byte_count,
                pending.preset.name,
                pending.preset.model,
                pending.destination
            ));
            app.llm_task = Some(RunningLlmRequest {
                task,
                preset_name: pending.preset.name,
                source_snapshot: pending.source_snapshot,
                path: pending.path,
                file_path: pending.file_path,
                replacement_target: pending.replacement_target,
                purpose: pending.purpose,
            });
        }
        Err(error) => app.message = Some(format!("Could not start LLM request: {error}")),
    }
    app.render(out)
}

fn cancel_pending(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    app.pending_llm_request = None;
    app.message = None;
    app.render(out)
}

fn collect_draft(
    app: &super::App,
    command: CurrentLlmCommand,
    instruction: &str,
    source: &str,
) -> Result<RequestDraft, context::ContextError> {
    let path = app.file.path.as_deref();
    match command {
        CurrentLlmCommand::Meow => collect_meow(app, instruction, source, path),
        CurrentLlmCommand::BigMeow => {
            let instruction = instruction_for_file(app, instruction, source, path)?;
            context::for_current_file(source, &instruction, path)
        }
    }
}

fn collect_meow(
    app: &super::App,
    instruction: &str,
    source: &str,
    path: Option<&Path>,
) -> Result<RequestDraft, context::ContextError> {
    if let Some(selection) = app.selection.active() {
        let (start, end) = selection.ordered();
        let text = app
            .buffer
            .text_range(start, end)
            .map_err(|_| context::ContextError::EmptyContext)?;
        return context::for_selection(&text, start.row, instruction, path);
    }
    let mut draft = context::for_instruction_block(source, app.buffer.cursor().row, path)?;
    if !instruction.trim().is_empty() {
        draft.instruction = instruction.trim().to_string();
    }
    Ok(draft)
}

fn instruction_for_file(
    app: &super::App,
    instruction: &str,
    source: &str,
    path: Option<&Path>,
) -> Result<String, context::ContextError> {
    if !instruction.trim().is_empty() {
        return Ok(instruction.trim().to_string());
    }
    context::for_instruction_block(source, app.buffer.cursor().row, path)
        .map(|draft| draft.instruction)
}

fn replacement_target(
    app: &super::App,
    draft: &RequestDraft,
) -> Option<super::llm_preview::RegionTarget> {
    if draft.context.scope != context::ContextScope::Selection {
        return None;
    }
    let selection = app.selection.active()?;
    let (start, end) = selection.ordered();
    Some(super::llm_preview::RegionTarget::new(
        start,
        end,
        draft.context.text.clone(),
    ))
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests;
