//! Purpose: this file must gate and start explicit Project repo-context preparation.
//! Owns: capability checks, current-file draft validation, config load, and prepare task start.
//! Must not: construct clients, read API keys, block for repo scans, network, or mutate buffers.
//! Invariants: Plain returns before constructing any broker task; preparation has no client.

use std::io::{self, Write};

use crate::config::llm::BackendPreset;
use crate::llm::context;
use crate::llm::repo_prepare::RepoPrepareTask;

use super::{Preparing, RepoLlmState};

pub(crate) fn begin(
    app: &mut super::super::App,
    out: &mut dyn Write,
    command: super::RepoLlmCommand,
    instruction: &str,
) -> io::Result<()> {
    if !app.caps.repo_llm || app.project.is_none() {
        app.message_info("Repo LLM requires explicit Project mode (:project).");
        return app.render(out);
    }
    let catalog = match crate::config::llm::load() {
        Ok(catalog) => catalog,
        Err(error) => {
            app.message_error(format!("LLM config error: {error}"));
            return app.render(out);
        }
    };
    let preset = app.model_session.effective(&catalog);
    begin_with_command_and_settings(app, out, command, instruction, preset)
}

#[cfg(test)]
pub(super) fn begin_with_settings(
    app: &mut super::super::App,
    out: &mut dyn Write,
    instruction: &str,
    preset: BackendPreset,
) -> io::Result<()> {
    begin_with_command_and_settings(
        app,
        out,
        super::RepoLlmCommand::GitMeow,
        instruction,
        preset,
    )
}

pub(super) fn begin_with_command_and_settings(
    app: &mut super::super::App,
    out: &mut dyn Write,
    command: super::RepoLlmCommand,
    instruction: &str,
    preset: BackendPreset,
) -> io::Result<()> {
    if app.repo_llm_state.is_some()
        || app.pending_llm_request.is_some()
        || app.llm_task.is_some()
        || super::super::inline_clanker::is_busy(app)
    {
        app.message_info("An LLM request is already pending or running.");
        return app.render(out);
    }
    if app.buffer.is_read_only() || app.buffer.page_info().is_some() {
        app.message_info("Repo LLM requires a fully editable active buffer.");
        return app.render(out);
    }
    let Some(path) = app.file.path.clone() else {
        app.message_info("Save the active buffer before using repo LLM.");
        return app.render(out);
    };
    let source_snapshot = app.buffer.to_string();
    let draft = match context::for_current_file(&source_snapshot, instruction, Some(&path)) {
        Ok(draft) => draft,
        Err(error) => {
            app.message_error(format!("Cannot prepare repo LLM request: {error}"));
            return app.render(out);
        }
    };
    let root = app.project.as_ref().expect("Project checked").root();
    match RepoPrepareTask::start_with_budget(root, &path, command.context_budget()) {
        Ok(task) => {
            app.repo_llm_state = Some(RepoLlmState::Preparing(Preparing {
                task,
                command,
                draft,
                preset,
                source_snapshot,
                path,
            }));
            app.message_info(format!(
                "Building {} {} repo context ({} KiB max)... Esc cancels; typing remains live.",
                command.name(),
                command.profile(),
                command.context_budget() / 1024
            ));
        }
        Err(error) => app.message_error(format!("Could not start repo context worker: {error}")),
    }
    app.render(out)
}
