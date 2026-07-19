//! Purpose: this file must cage Project-only repo-aware LLM commands end to end.
//! Owns: async context preparation, explicit send confirmation, task polling, and cancellation.
//! Must not: construct in Plain, block typing, apply output, write files, or bypass repo checks.
//! Invariants: no client before Enter; source/path/repo drift refuses preview and apply.
//! Phase: 6 (LLM Context Broker).

use std::io::{self, Write};
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::llm::LlmSettings;
use crate::llm::context::RequestDraft;
use crate::llm::openai_compat::LlmConfig;
use crate::llm::repo_prepare::{PreparedRepoContext, RepoPrepareResult, RepoPrepareTask};
use crate::llm::repo_task::RepoLlmTask;

const SYSTEM_PROMPT: &str = "You edit only the named active file using read-only repository context. You may request more context by returning exactly {\"catomic_broker\":{\"command\":\"list_files\"}}, {\"catomic_broker\":{\"command\":\"read_file\",\"path\":\"relative/path\",\"offset\":0,\"limit\":4096}}, {\"catomic_broker\":{\"command\":\"grep\",\"query\":\"text\"}}, or {\"catomic_broker\":{\"command\":\"show_diff\",\"path\":\"relative/path\"}}. Your final response must be one valid single-file unified diff for the active file, with no prose or fences. Never claim an edit was applied.";

const FOCUSED_REPO_CONTEXT_BUDGET: usize = 64 * 1024;
const BROAD_REPO_CONTEXT_BUDGET: usize = crate::llm::broker::DEFAULT_CONTEXT_BUDGET;

mod apply_check;
mod checking;
mod result;
mod start;

pub(crate) use start::begin;
#[cfg(test)]
use start::{begin_with_command_and_settings, begin_with_settings};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RepoLlmCommand {
    GitMeow,
    MegaMeow,
}

impl RepoLlmCommand {
    fn name(self) -> &'static str {
        match self {
            Self::GitMeow => "gitmeow",
            Self::MegaMeow => "megameow",
        }
    }

    fn profile(self) -> &'static str {
        match self {
            Self::GitMeow => "focused",
            Self::MegaMeow => "broader",
        }
    }

    fn context_budget(self) -> usize {
        match self {
            Self::GitMeow => FOCUSED_REPO_CONTEXT_BUDGET,
            Self::MegaMeow => BROAD_REPO_CONTEXT_BUDGET,
        }
    }
}

pub(crate) enum RepoLlmState {
    Preparing(Preparing),
    Pending(Box<Pending>),
    CheckingSend(checking::CheckingSend),
    Running(Running),
    CheckingApply(apply_check::CheckingApply),
}

pub(crate) struct Preparing {
    task: RepoPrepareTask,
    command: RepoLlmCommand,
    draft: RequestDraft,
    settings: LlmSettings,
    source_snapshot: String,
    path: PathBuf,
}

pub(crate) struct Pending {
    prepared: PreparedRepoContext,
    command: RepoLlmCommand,
    draft: RequestDraft,
    settings: LlmSettings,
    source_snapshot: String,
    file_path: PathBuf,
    relative_path: String,
}

pub(crate) struct Running {
    task: RepoLlmTask,
    source_snapshot: String,
    file_path: PathBuf,
    relative_path: String,
}

pub(crate) fn poll(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if matches!(
        app.repo_llm_state.as_ref(),
        Some(RepoLlmState::Preparing(_))
    ) {
        return poll_preparing(app, out);
    }
    if matches!(
        app.repo_llm_state.as_ref(),
        Some(RepoLlmState::CheckingSend(_))
    ) {
        return checking::poll(app, out);
    }
    if matches!(app.repo_llm_state.as_ref(), Some(RepoLlmState::Running(_))) {
        return result::poll_running(app, out);
    }
    if matches!(
        app.repo_llm_state.as_ref(),
        Some(RepoLlmState::CheckingApply(_))
    ) {
        return apply_check::poll(app, out);
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
        RepoPrepareResult::Finished(prepared) => finish_preparing(app, *prepared, state),
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
    if app.file.path.as_ref() != Some(&state.path) {
        app.message = Some(
            "Active file path changed while repo context was built; request cancelled.".to_string(),
        );
        return;
    }
    let relative_path = prepared.active_relative_path.clone();
    let context_kib = state.command.context_budget() / 1024;
    let sensitive = if state.draft.context.sensitivity.is_empty() {
        ""
    } else {
        " SENSITIVE active-file context detected; Enter explicitly allows it."
    };
    app.message = Some(format!(
        "{} at {}: send {} {} context with {} initial repo bytes + {} active-file bytes (at most {context_kib} KiB repository context total)?{sensitive} Enter confirms; Esc cancels.",
        state.settings.model, state.settings.base_url, state.command.name(), state.command.profile(), prepared.initial_context.len(), state.draft.context.byte_count
    ));
    app.repo_llm_state = Some(RepoLlmState::Pending(Box::new(Pending {
        prepared,
        command: state.command,
        draft: state.draft,
        settings: state.settings,
        source_snapshot: state.source_snapshot,
        file_path: state.path,
        relative_path,
    })));
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
                KeyCode::Enter => checking::begin(app),
                KeyCode::Esc => cancel_pending(app),
                _ => {
                    app.message =
                        Some("Repo LLM send not confirmed. Enter sends; Esc cancels.".to_string())
                }
            }
            app.render(out)?;
            Ok(true)
        }
        Some(RepoLlmState::CheckingSend(_)) if is_quit(key) => Ok(false),
        Some(RepoLlmState::CheckingSend(_)) => {
            checking::handle_key(app, key);
            app.render(out)?;
            Ok(true)
        }
        Some(RepoLlmState::CheckingApply(_)) if is_quit(key) => Ok(false),
        Some(RepoLlmState::CheckingApply(_)) => {
            apply_check::handle_key(app, key);
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
    let message = match app.repo_llm_state.as_ref() {
        Some(RepoLlmState::Pending(_)) => "Repo LLM send not confirmed. Enter sends; Esc cancels.",
        Some(RepoLlmState::CheckingSend(_)) => "Repository check running; Esc cancels.",
        Some(RepoLlmState::CheckingApply(_)) => {
            "Final repository check running; Esc cancels the proposal."
        }
        _ => return Ok(false),
    };
    app.message = Some(message.to_string());
    app.render(out)?;
    Ok(true)
}

pub(crate) fn begin_apply_check(
    app: &mut super::App,
    broker: crate::llm::broker::ContextBroker,
) -> io::Result<()> {
    apply_check::begin(app, broker)
}

pub(super) fn start_confirmed(app: &mut super::App, pending: Pending) {
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
                "Sending {} {} repo context to {} at {}... Esc cancels.",
                pending.command.name(),
                pending.command.profile(),
                pending.settings.model,
                pending.settings.base_url
            ));
            app.repo_llm_state = Some(RepoLlmState::Running(Running {
                task,
                source_snapshot: pending.source_snapshot,
                file_path: pending.file_path,
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
        "Command: {} ({})\nActive path: {}\nInstruction:\n{}\n\nActive file:\n{}\n\nBounded repository context:\n{}",
        pending.command.name(),
        pending.command.profile(),
        pending.relative_path,
        pending.draft.instruction,
        pending.draft.context.text,
        pending.prepared.initial_context
    )
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests;
