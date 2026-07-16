//! Purpose: this file must gate and start explicit Project repo-context preparation.
//! Owns: capability checks, current-file draft validation, config load, and prepare task start.
//! Must not: construct clients, read API keys, block for repo scans, network, or mutate buffers.
//! Invariants: Plain returns before constructing any broker task; preparation has no client.
//! Phase: 6 (LLM Context Broker).

use std::io::{self, Write};

use crate::config::llm::LlmSettings;
use crate::llm::context;
use crate::llm::repo_prepare::RepoPrepareTask;

use super::{Preparing, RepoLlmState};

pub(crate) fn begin(
    app: &mut super::super::App,
    out: &mut dyn Write,
    instruction: &str,
) -> io::Result<()> {
    if !app.caps.repo_llm || app.project.is_none() {
        app.message = Some("Repo LLM requires explicit Project mode (:project).".to_string());
        return app.render(out);
    }
    let settings = match crate::config::llm::load() {
        Ok(settings) => settings,
        Err(error) => {
            app.message = Some(format!("LLM config error: {error}"));
            return app.render(out);
        }
    };
    begin_with_settings(app, out, instruction, settings)
}

pub(super) fn begin_with_settings(
    app: &mut super::super::App,
    out: &mut dyn Write,
    instruction: &str,
    settings: LlmSettings,
) -> io::Result<()> {
    if app.repo_llm_state.is_some() || app.pending_llm_request.is_some() || app.llm_task.is_some() {
        app.message = Some("An LLM request is already pending or running.".to_string());
        return app.render(out);
    }
    if app.buffer.is_read_only() || app.buffer.page_info().is_some() {
        app.message = Some("Repo LLM requires a fully editable active buffer.".to_string());
        return app.render(out);
    }
    let Some(path) = app.file.path.clone() else {
        app.message = Some("Save the active buffer before using repo LLM.".to_string());
        return app.render(out);
    };
    let source_snapshot = app.buffer.to_string();
    let draft = match context::for_current_file(&source_snapshot, instruction, Some(&path)) {
        Ok(draft) => draft,
        Err(error) => {
            app.message = Some(format!("Cannot prepare repo LLM request: {error}"));
            return app.render(out);
        }
    };
    let root = app.project.as_ref().expect("Project checked").root();
    match RepoPrepareTask::start(root, &path) {
        Ok(task) => {
            app.repo_llm_state = Some(RepoLlmState::Preparing(Preparing {
                task,
                draft,
                settings,
                source_snapshot,
                path,
            }));
            app.message = Some(
                "Building bounded repo context... Esc cancels; typing remains live.".to_string(),
            );
        }
        Err(error) => app.message = Some(format!("Could not start repo context worker: {error}")),
    }
    app.render(out)
}
