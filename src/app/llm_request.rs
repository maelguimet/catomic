//! Purpose: this file must cage explicit current-buffer LLM invocation end to end.
//! Owns: `:meow` drafts, endpoint/context confirmation, task polling, and cancellation.
//! Must not: collect repo context, create clients before Enter, apply output, or write files.
//! Invariants: pending state has no client; source drift discards output; patches go to preview.
//! Phase: 6 (LLM, Powerful but Caged).

use std::io::{self, Write};
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::llm::LlmSettings;
use crate::llm::context::{self, RequestDraft};
use crate::llm::openai_compat::LlmConfig;
use crate::llm::task::{LlmTask, LlmTaskResult};

mod prompt;

use prompt::{confirmation_message, display_path, user_prompt, SYSTEM_PROMPT};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CurrentLlmCommand {
    Meow,
    BigMeow,
}

pub(crate) struct PendingLlmRequest {
    draft: RequestDraft,
    settings: LlmSettings,
    source_snapshot: String,
    path: String,
}

pub(crate) struct RunningLlmRequest {
    task: LlmTask,
    source_snapshot: String,
}

pub(crate) fn begin(
    app: &mut super::App,
    out: &mut dyn Write,
    command: CurrentLlmCommand,
    instruction: &str,
) -> io::Result<()> {
    let settings = match crate::config::llm::load() {
        Ok(settings) => settings,
        Err(error) => {
            app.message = Some(format!("LLM config error: {error}"));
            return app.render(out);
        }
    };
    begin_with_settings(app, out, command, instruction, settings)
}

fn begin_with_settings(
    app: &mut super::App,
    out: &mut dyn Write,
    command: CurrentLlmCommand,
    instruction: &str,
    settings: LlmSettings,
) -> io::Result<()> {
    if app.pending_llm_request.is_some() || app.llm_task.is_some() {
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
    let path = display_path(app.file.path.as_deref());
    app.message = Some(confirmation_message(&draft, &settings));
    app.pending_llm_request = Some(PendingLlmRequest {
        draft,
        settings,
        source_snapshot,
        path,
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
        app.message = Some("LLM request cancelled.".to_string());
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

pub(crate) fn poll(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let result = app
        .llm_task
        .as_mut()
        .and_then(|running| running.task.try_result());
    let Some(result) = result else {
        return Ok(());
    };
    let running = app.llm_task.take().expect("completed task exists");
    match result {
        LlmTaskResult::Finished(output) => {
            if app.buffer.to_string() != running.source_snapshot {
                app.message = Some(
                    "Buffer changed while the model was working; response was not previewed."
                        .to_string(),
                );
                app.render(out)
            } else {
                super::llm_preview::show(app, out, &output)
            }
        }
        LlmTaskResult::Cancelled => {
            app.message = Some("LLM request cancelled.".to_string());
            app.render(out)
        }
        LlmTaskResult::Error(error) => {
            app.message = Some(format!("LLM request failed: {error}"));
            app.render(out)
        }
    }
}

pub(crate) fn cancel_all(app: &mut super::App) -> bool {
    let pending = app.pending_llm_request.take().is_some();
    let running = app.llm_task.take().is_some();
    pending || running
}

fn confirm(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let pending = app.pending_llm_request.take().expect("pending request");
    if app.buffer.to_string() != pending.source_snapshot {
        app.message =
            Some("Buffer changed before confirmation; LLM request cancelled.".to_string());
        return app.render(out);
    }
    let api_key = std::env::var(&pending.settings.api_key_env)
        .ok()
        .filter(|key| !key.is_empty());
    let config = LlmConfig {
        base_url: pending.settings.base_url.clone(),
        api_key,
        model: pending.settings.model.clone(),
        timeout: pending.settings.timeout,
    };
    let user = user_prompt(&pending.draft, &pending.path);
    match LlmTask::start(config, SYSTEM_PROMPT.to_string(), user) {
        Ok(task) => {
            app.message = Some(format!(
                "Sending {} lines/{} bytes to {} at {}... Esc cancels.",
                pending.draft.context.line_count,
                pending.draft.context.byte_count,
                pending.settings.model,
                pending.settings.base_url
            ));
            app.llm_task = Some(RunningLlmRequest {
                task,
                source_snapshot: pending.source_snapshot,
            });
        }
        Err(error) => app.message = Some(format!("Could not start LLM request: {error}")),
    }
    app.render(out)
}

fn cancel_pending(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    app.pending_llm_request = None;
    app.message = Some("LLM request cancelled before sending; no network call made.".to_string());
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

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests;
