//! Purpose: this file must cage Project-only repo-aware LLM commands end to end.
//! Owns: async context preparation, explicit send confirmation, task polling, and cancellation.
//! Must not: construct in Plain, block typing, apply output, write files, or bypass repo checks.
//! Invariants: no client before Enter; source/repo drift refuses preview and confirmed apply.
//! Phase: 6 (LLM Context Broker).

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::llm::LlmSettings;
use crate::llm::context::RequestDraft;
use crate::llm::openai_compat::LlmConfig;
use crate::llm::repo_prepare::{PreparedRepoContext, RepoPrepareResult, RepoPrepareTask};
use crate::llm::repo_task::RepoLlmTask;

const SYSTEM_PROMPT: &str = "You edit only the named active file using read-only repository context. You may request more context by returning exactly {\"catomic_broker\":{\"command\":\"list_files\"}}, {\"catomic_broker\":{\"command\":\"read_file\",\"path\":\"relative/path\",\"offset\":0,\"limit\":4096}}, {\"catomic_broker\":{\"command\":\"grep\",\"query\":\"text\"}}, or {\"catomic_broker\":{\"command\":\"show_diff\",\"path\":\"relative/path\"}}. Your final response must be one valid single-file unified diff for the active file, with no prose or fences. Never claim an edit was applied.";

mod result;
mod start;

pub(crate) use start::begin;
#[cfg(test)]
use start::begin_with_settings;

pub(crate) enum RepoLlmState {
    Preparing(Preparing),
    Pending(Pending),
    Running(Running),
}

pub(crate) struct Preparing {
    task: RepoPrepareTask,
    draft: RequestDraft,
    settings: LlmSettings,
    source_snapshot: String,
    path: PathBuf,
}

pub(crate) struct Pending {
    prepared: PreparedRepoContext,
    draft: RequestDraft,
    settings: LlmSettings,
    source_snapshot: String,
    relative_path: String,
}

pub(crate) struct Running {
    task: RepoLlmTask,
    source_snapshot: String,
    relative_path: String,
}

pub(crate) fn poll(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if matches!(
        app.repo_llm_state.as_ref(),
        Some(RepoLlmState::Preparing(_))
    ) {
        return poll_preparing(app, out);
    }
    if matches!(app.repo_llm_state.as_ref(), Some(RepoLlmState::Running(_))) {
        return result::poll_running(app, out);
    }
    Ok(())
}

fn poll_preparing(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let result = match app.repo_llm_state.as_mut() {
        Some(RepoLlmState::Preparing(state)) => state.task.try_result(),
        _ => None,
    };
    let Some(result) = result else {
        return Ok(());
    };
    let RepoLlmState::Preparing(state) = app.repo_llm_state.take().unwrap() else {
        unreachable!()
    };
    match result {
        RepoPrepareResult::Finished(prepared) => finish_preparing(app, prepared, state),
        RepoPrepareResult::Cancelled => {
            app.message = Some("Repo context preparation cancelled.".to_string())
        }
        RepoPrepareResult::Error(error) => {
            app.message = Some(format!("Repo context error: {error}"))
        }
    }
    app.render(out)
}

fn finish_preparing(app: &mut super::App, prepared: PreparedRepoContext, state: Preparing) {
    if app.buffer.to_string() != state.source_snapshot {
        app.message = Some(
            "Active buffer changed while repo context was built; request cancelled.".to_string(),
        );
        return;
    }
    let relative_path = match repo_relative_path(&prepared.broker.git.root, &state.path) {
        Ok(path) => path,
        Err(message) => {
            app.message = Some(message);
            return;
        }
    };
    let sensitive = if state.draft.context.sensitivity.is_empty() {
        ""
    } else {
        " SENSITIVE active-file context detected; Enter explicitly allows it."
    };
    app.message = Some(format!(
        "Send {} repo bytes + {} active-file bytes to {} at {}?{sensitive} Enter confirms; Esc cancels.",
        prepared.initial_context.len(), state.draft.context.byte_count, state.settings.model, state.settings.base_url
    ));
    app.repo_llm_state = Some(RepoLlmState::Pending(Pending {
        prepared,
        draft: state.draft,
        settings: state.settings,
        source_snapshot: state.source_snapshot,
        relative_path,
    }));
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    match app.repo_llm_state.as_ref() {
        Some(RepoLlmState::Pending(_)) if is_quit(key) => Ok(false),
        Some(RepoLlmState::Pending(_)) => {
            match key.code {
                KeyCode::Enter => confirm(app),
                KeyCode::Esc => cancel_pending(app),
                _ => {
                    app.message =
                        Some("Repo LLM send not confirmed. Enter sends; Esc cancels.".to_string())
                }
            }
            app.render(out)?;
            Ok(true)
        }
        Some(RepoLlmState::Preparing(_) | RepoLlmState::Running(_)) if key.code == KeyCode::Esc => {
            cancel_all(app);
            app.message = Some("Repo LLM request cancelled.".to_string());
            app.render(out)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub(crate) fn handle_paste(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    if !matches!(app.repo_llm_state.as_ref(), Some(RepoLlmState::Pending(_))) {
        return Ok(false);
    }
    app.message = Some("Repo LLM send not confirmed. Enter sends; Esc cancels.".to_string());
    app.render(out)?;
    Ok(true)
}

fn confirm(app: &mut super::App) {
    let Some(RepoLlmState::Pending(pending)) = app.repo_llm_state.take() else {
        return;
    };
    if app.buffer.to_string() != pending.source_snapshot {
        app.message =
            Some("Active buffer changed before confirmation; repo LLM cancelled.".to_string());
        return;
    }
    match pending.prepared.broker.is_unchanged() {
        Ok(true) => {}
        Ok(false) => {
            app.message =
                Some("Repository changed before confirmation; repo LLM cancelled.".to_string());
            return;
        }
        Err(error) => {
            app.message = Some(format!("Could not recheck repository: {error}"));
            return;
        }
    }
    let config = llm_config(&pending.settings);
    let user = user_prompt(&pending);
    match RepoLlmTask::start(
        config,
        pending.prepared.broker,
        SYSTEM_PROMPT.to_string(),
        user,
    ) {
        Ok(task) => {
            app.message = Some(format!(
                "Sending repo context to {} at {}... Esc cancels.",
                pending.settings.model, pending.settings.base_url
            ));
            app.repo_llm_state = Some(RepoLlmState::Running(Running {
                task,
                source_snapshot: pending.source_snapshot,
                relative_path: pending.relative_path,
            }));
        }
        Err(error) => app.message = Some(format!("Could not start repo LLM request: {error}")),
    }
}

pub(crate) fn cancel_all(app: &mut super::App) -> bool {
    app.repo_llm_state.take().is_some()
}

fn cancel_pending(app: &mut super::App) {
    app.repo_llm_state = None;
    app.message = Some("Repo LLM cancelled before sending; no network call made.".to_string());
}

fn llm_config(settings: &LlmSettings) -> LlmConfig {
    LlmConfig {
        base_url: settings.base_url.clone(),
        api_key: std::env::var(&settings.api_key_env)
            .ok()
            .filter(|key| !key.is_empty()),
        model: settings.model.clone(),
        timeout: settings.timeout,
    }
}

fn user_prompt(pending: &Pending) -> String {
    format!(
        "Active path: {}\nInstruction:\n{}\n\nActive file:\n{}\n\nBounded repository context:\n{}",
        pending.relative_path,
        pending.draft.instruction,
        pending.draft.context.text,
        pending.prepared.initial_context
    )
}

fn repo_relative_path(root: &Path, path: &Path) -> Result<String, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(path)
    };
    let canonical = absolute
        .canonicalize()
        .map_err(|error| format!("Cannot resolve active repo file: {error}"))?;
    canonical
        .strip_prefix(root)
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|_| "Active file is outside the detected Git repository.".to_string())
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests;
