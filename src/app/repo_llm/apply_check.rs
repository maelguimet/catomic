//! Purpose: this file must recheck repository drift before final preview apply.
//! Owns: the final apply-check state, non-blocking polling, and guarded apply handoff.
//! Must not: run Git on the input thread, contact endpoints, write files, or apply directly.
//! Invariants: only an unchanged result may reach the ordinary preview transaction path.
//! Phase: 6 acceptance hardening.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent};

use crate::llm::broker::ContextBroker;
use crate::llm::repo_check::{RepoCheckResult, RepoCheckTask};

use super::RepoLlmState;

pub(crate) struct CheckingApply {
    task: RepoCheckTask,
}

pub(super) fn begin(app: &mut super::super::App, broker: ContextBroker) -> io::Result<()> {
    if app.repo_llm_state.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "another repo LLM task is active",
        ));
    }
    let task = RepoCheckTask::start(broker)?;
    app.repo_llm_state = Some(RepoLlmState::CheckingApply(CheckingApply { task }));
    Ok(())
}

pub(super) fn poll(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    let result = match app.repo_llm_state.as_mut() {
        Some(RepoLlmState::CheckingApply(state)) => state.task.try_result(),
        _ => None,
    };
    let Some(result) = result else {
        return Ok(());
    };
    let RepoLlmState::CheckingApply(_) = app.repo_llm_state.take().unwrap() else {
        unreachable!()
    };
    match result {
        RepoCheckResult::Unchanged(_) => super::super::llm_preview::finish_repo_apply(app, out),
        RepoCheckResult::Changed => refuse(
            app,
            out,
            "Repository changed since the request; repo LLM patch was not applied.",
        ),
        RepoCheckResult::Cancelled => refuse(
            app,
            out,
            "Final repository check cancelled; no changes applied.",
        ),
        RepoCheckResult::Error(error) => refuse(
            app,
            out,
            &format!("Could not recheck repository; patch refused: {error}"),
        ),
    }
}

pub(super) fn handle_key(app: &mut super::super::App, key: KeyEvent) {
    if key.code == KeyCode::Esc {
        super::cancel_all(app);
        super::super::llm_preview::close(app);
        app.message = None;
        app.reveal_cursor();
    } else {
        app.message_info("Final repository check running; Esc cancels the proposal.");
    }
}

fn refuse(app: &mut super::super::App, out: &mut dyn Write, message: &str) -> io::Result<()> {
    super::super::llm_preview::close(app);
    app.message_warning(message);
    app.reveal_cursor();
    app.render(out)
}
