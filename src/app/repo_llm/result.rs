//! Purpose: this file must finish confirmed repo LLM tasks without weakening drift checks.
//! Owns: completed-task polling, source/path rechecks, and guarded preview handoff.
//! Must not: construct clients, read repos, apply edits, write files, or accept stale responses.
//! Invariants: only an unchanged source identity and repository can reach patch preview.
//! Phase: 6 (LLM Context Broker).

use std::io::{self, Write};
use std::path::Path;

use crate::llm::broker::ContextBroker;
use crate::llm::repo_task::RepoLlmTaskResult;

use super::RepoLlmState;

pub(super) fn poll_running(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    let result = match app.repo_llm_state.as_mut() {
        Some(RepoLlmState::Running(state)) => state.task.try_result(),
        _ => None,
    };
    let Some(result) = result else {
        return Ok(());
    };
    let RepoLlmState::Running(state) = app.repo_llm_state.take().unwrap() else {
        unreachable!()
    };
    match result {
        RepoLlmTaskResult::Finished { output, broker } => finish_output(
            app,
            out,
            output,
            broker,
            &state.source_snapshot,
            &state.file_path,
            &state.relative_path,
        ),
        RepoLlmTaskResult::RepositoryChanged => render_message(
            app,
            out,
            "Repository changed while repo model worked; response discarded.",
        ),
        RepoLlmTaskResult::RepositoryCheckFailed(error) => render_message(
            app,
            out,
            &format!("Could not recheck repository; response discarded: {error}"),
        ),
        RepoLlmTaskResult::Cancelled => render_message(app, out, "Repo LLM request cancelled."),
        RepoLlmTaskResult::Error(error) => {
            render_message(app, out, &format!("Repo LLM request failed: {error}"))
        }
    }
}

fn finish_output(
    app: &mut super::super::App,
    out: &mut dyn Write,
    output: String,
    broker: ContextBroker,
    source_snapshot: &str,
    file_path: &Path,
    expected_path: &str,
) -> io::Result<()> {
    if app.buffer.to_string() != source_snapshot {
        return render_message(
            app,
            out,
            "Active buffer changed while repo model worked; response discarded.",
        );
    }
    if app.file.path.as_deref() != Some(file_path) {
        return render_message(
            app,
            out,
            "Active file path changed while repo model worked; response discarded.",
        );
    }
    super::super::llm_preview::show_repo_patch(app, out, &output, expected_path, broker)
}

fn render_message(
    app: &mut super::super::App,
    out: &mut dyn Write,
    message: &str,
) -> io::Result<()> {
    app.message = Some(message.to_string());
    app.render(out)
}
