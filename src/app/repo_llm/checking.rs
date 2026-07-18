//! Purpose: this file must recheck repository drift before an explicit LLM send.
//! Owns: the pre-send worker state, non-blocking polling, and guarded handoff to HTTP.
//! Must not: run Git on the input thread, contact endpoints before success, or edit buffers.
//! Invariants: source, path, and repository identity remain pinned across the async check.
//! Phase: 6 acceptance hardening.

use std::io::{self, Write};
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};

use crate::config::llm::LlmSettings;
use crate::llm::broker::ContextBroker;
use crate::llm::context::RequestDraft;
use crate::llm::repo_check::{RepoCheckResult, RepoCheckTask};
use crate::llm::repo_prepare::PreparedRepoContext;

use super::{Pending, RepoLlmCommand, RepoLlmState};

pub(crate) struct CheckingSend {
    task: RepoCheckTask,
    command: RepoLlmCommand,
    draft: RequestDraft,
    settings: LlmSettings,
    source_snapshot: String,
    file_path: PathBuf,
    relative_path: String,
    initial_context: String,
}

pub(super) fn begin(app: &mut super::super::App) {
    let Some(RepoLlmState::Pending(pending)) = app.repo_llm_state.take() else {
        return;
    };
    if app.buffer.to_string() != pending.source_snapshot {
        app.message =
            Some("Active buffer changed before confirmation; repo LLM cancelled.".to_string());
        return;
    }
    if app.file.path.as_ref() != Some(&pending.file_path) {
        app.message =
            Some("Active file path changed before confirmation; repo LLM cancelled.".to_string());
        return;
    }
    start(app, *pending);
}

fn start(app: &mut super::super::App, pending: Pending) {
    let PreparedRepoContext {
        broker,
        initial_context,
        ..
    } = pending.prepared;
    match RepoCheckTask::start(broker) {
        Ok(task) => {
            app.message = Some("Rechecking repository before send... Esc cancels.".to_string());
            app.repo_llm_state = Some(RepoLlmState::CheckingSend(CheckingSend {
                task,
                command: pending.command,
                draft: pending.draft,
                settings: pending.settings,
                source_snapshot: pending.source_snapshot,
                file_path: pending.file_path,
                relative_path: pending.relative_path,
                initial_context,
            }));
        }
        Err(error) => app.message = Some(format!("Could not start repository check: {error}")),
    }
}

pub(super) fn poll(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    let result = match app.repo_llm_state.as_mut() {
        Some(RepoLlmState::CheckingSend(state)) => state.task.try_result(),
        _ => None,
    };
    let Some(result) = result else {
        return Ok(());
    };
    let RepoLlmState::CheckingSend(state) = app.repo_llm_state.take().unwrap() else {
        unreachable!()
    };
    match result {
        RepoCheckResult::Unchanged(broker) => finish(app, *broker, state),
        RepoCheckResult::Changed => {
            app.message =
                Some("Repository changed before confirmation; repo LLM cancelled.".to_string())
        }
        RepoCheckResult::Cancelled => {
            app.message = Some("Repository check cancelled; no network call made.".to_string())
        }
        RepoCheckResult::Error(error) => {
            app.message = Some(format!("Could not recheck repository: {error}"))
        }
    }
    app.render(out)
}

fn finish(app: &mut super::super::App, broker: ContextBroker, state: CheckingSend) {
    if app.buffer.to_string() != state.source_snapshot {
        app.message =
            Some("Active buffer changed during repository check; repo LLM cancelled.".to_string());
        return;
    }
    if app.file.path.as_ref() != Some(&state.file_path) {
        app.message = Some(
            "Active file path changed during repository check; repo LLM cancelled.".to_string(),
        );
        return;
    }
    super::start_confirmed(app, pending(broker, state));
}

fn pending(broker: ContextBroker, state: CheckingSend) -> Pending {
    Pending {
        prepared: PreparedRepoContext {
            broker,
            initial_context: state.initial_context,
            active_relative_path: state.relative_path.clone(),
        },
        command: state.command,
        draft: state.draft,
        settings: state.settings,
        source_snapshot: state.source_snapshot,
        file_path: state.file_path,
        relative_path: state.relative_path,
    }
}

pub(super) fn handle_key(app: &mut super::super::App, key: KeyEvent) {
    if key.code == KeyCode::Esc {
        super::cancel_all(app);
        app.message = Some("Repository check cancelled; no network call made.".to_string());
    } else {
        app.message = Some("Repository check running; Esc cancels.".to_string());
    }
}
