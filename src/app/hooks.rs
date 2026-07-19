//! Purpose: sequence configured lifecycle commands and resume deferred LLM preparation.
//! Owns: hook queues, active-hook outcome, lifecycle triggers, and before-LLM continuation.
//! Must not: spawn processes directly, apply output, load config, write files, or call network.
//! Invariants: hooks run in configured order; failure/cancellation aborts the remaining chain.
//! Phase: 7 lifecycle hooks.

use std::collections::VecDeque;
use std::io::{self, Write};

use crate::config::commands::HookEvent;

#[derive(Default)]
pub(crate) struct HookState {
    queue: VecDeque<String>,
    active: Option<String>,
    continuation: Option<Continuation>,
}

enum Continuation {
    CurrentLlm {
        command: super::llm_request::CurrentLlmCommand,
        instruction: String,
    },
    RepoLlm {
        command: super::repo_llm::RepoLlmCommand,
        instruction: String,
    },
    InlineClanker,
}

pub(crate) fn trigger_open(app: &mut super::App) {
    if app.file.path.is_some() {
        enqueue(app, HookEvent::Open);
    }
}

pub(crate) fn trigger_save(app: &mut super::App) {
    let has_hooks = !app.command_config.hooks_for(HookEvent::Save).is_empty();
    enqueue(app, HookEvent::Save);
    if has_hooks {
        super::save_trace::note_hook(app, "queued", None);
    }
}

pub(crate) fn before_current_llm(
    app: &mut super::App,
    out: &mut dyn Write,
    command: super::llm_request::CurrentLlmCommand,
    instruction: &str,
) -> io::Result<()> {
    let continuation = Continuation::CurrentLlm {
        command,
        instruction: instruction.to_string(),
    };
    begin_before_llm(app, out, continuation)
}

pub(crate) fn before_repo_llm(
    app: &mut super::App,
    out: &mut dyn Write,
    command: super::repo_llm::RepoLlmCommand,
    instruction: &str,
) -> io::Result<()> {
    let continuation = Continuation::RepoLlm {
        command,
        instruction: instruction.to_string(),
    };
    begin_before_llm(app, out, continuation)
}

pub(crate) fn before_inline_clanker(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    begin_before_llm(app, out, Continuation::InlineClanker)
}

pub(crate) fn pump(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if app.hooks.active.is_some() || super::external_command::is_busy(app) {
        return Ok(());
    }
    if let Some(name) = app.hooks.queue.pop_front() {
        super::save_trace::note_hook(app, "started", Some(&name));
        app.hooks.active = Some(name.clone());
        if !super::external_command::start_hook(app, out, &name)? {
            finish_command(app, false);
            app.render(out)?;
        }
        return Ok(());
    }
    let Some(continuation) = app.hooks.continuation.take() else {
        return Ok(());
    };
    match continuation {
        Continuation::CurrentLlm {
            command,
            instruction,
        } => super::llm_request::begin(app, out, command, &instruction),
        Continuation::RepoLlm {
            command,
            instruction,
        } => super::repo_llm::begin(app, out, command, &instruction),
        Continuation::InlineClanker => super::inline_clanker::begin(app, out),
    }
}

pub(crate) fn finish_command(app: &mut super::App, succeeded: bool) -> bool {
    let Some(name) = app.hooks.active.take() else {
        return false;
    };
    super::save_trace::note_hook(
        app,
        if succeeded { "succeeded" } else { "failed" },
        Some(&name),
    );
    if !succeeded {
        app.hooks.queue.clear();
        app.hooks.continuation = None;
        app.message = Some(format!(
            "Hook command {name} failed or was cancelled; chain stopped."
        ));
    }
    true
}

pub(crate) fn cancel_all(app: &mut super::App) -> bool {
    let active = app.hooks.active.take().is_some();
    let queued = !app.hooks.queue.is_empty();
    let continuation = app.hooks.continuation.take().is_some();
    app.hooks.queue.clear();
    active || queued || continuation
}

fn begin_before_llm(
    app: &mut super::App,
    out: &mut dyn Write,
    continuation: Continuation,
) -> io::Result<()> {
    if app.hooks.continuation.is_some() {
        app.message = Some("A before-LLM hook chain is already pending.".to_string());
        return app.render(out);
    }
    let names = app.command_config.hooks_for(HookEvent::BeforeLlm).to_vec();
    if names.is_empty() {
        return match continuation {
            Continuation::CurrentLlm {
                command,
                instruction,
            } => super::llm_request::begin(app, out, command, &instruction),
            Continuation::RepoLlm {
                command,
                instruction,
            } => super::repo_llm::begin(app, out, command, &instruction),
            Continuation::InlineClanker => super::inline_clanker::begin(app, out),
        };
    }
    app.hooks.queue.extend(names);
    app.hooks.continuation = Some(continuation);
    app.message = Some("Before-LLM hooks queued; Escape cancels the active command.".to_string());
    app.render(out)
}

fn enqueue(app: &mut super::App, event: HookEvent) {
    app.hooks
        .queue
        .extend(app.command_config.hooks_for(event).iter().cloned());
}

#[cfg(test)]
pub(crate) fn is_pending(app: &super::App) -> bool {
    app.hooks.active.is_some() || !app.hooks.queue.is_empty() || app.hooks.continuation.is_some()
}

#[cfg(test)]
mod tests;
