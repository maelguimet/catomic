//! Purpose: start and poll one bounded autocomplete request without blocking input.
//! Owns: debounce, active-buffer identity pins, task handoff, stale checks, and backoff.
//! Must not: confirm opt-in, render ghost layout, read files/repositories, or apply edits.
//! Invariants: one task at a time; late responses display only for the exact pinned identity.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use crate::llm::backend::ConfirmedBackend;
use crate::llm::task::{LlmTask, LlmTaskResult};

use super::{ConfirmedPolicy, RequestIdentity, RunningRequest, Suggestion};

pub(crate) fn poll(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    if let Some((identity, result)) = completed_result(app) {
        finish_result(app, out, identity, result)?;
    }
    if should_start(app, Instant::now()) {
        start(app, out)?;
    }
    Ok(())
}

fn completed_result(app: &mut super::super::App) -> Option<(RequestIdentity, LlmTaskResult)> {
    let result = app
        .autocomplete
        .running
        .as_mut()
        .and_then(|running| running.task.try_result())?;
    let running = app
        .autocomplete
        .running
        .take()
        .expect("completed autocomplete task");
    Some((running.identity, result))
}

fn should_start(app: &super::super::App, now: Instant) -> bool {
    if !app.autocomplete.enabled
        || app.autocomplete.running.is_some()
        || app.autocomplete.suggestion.is_some()
        || app.autocomplete.pending.is_some()
    {
        return false;
    }
    if app
        .autocomplete
        .backoff_until
        .is_some_and(|deadline| now < deadline)
    {
        return false;
    }
    let Some(last_edit) = app.autocomplete.last_edit else {
        return false;
    };
    let Some(policy) = app.autocomplete.confirmed.as_ref() else {
        return false;
    };
    now.duration_since(last_edit) >= policy.autocomplete.idle_debounce
}

fn start(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    if app.buffer.is_read_only()
        || app.buffer.page_info().is_some()
        || app.selection.active().is_some()
    {
        app.autocomplete.last_edit = None;
        return Ok(());
    }
    let policy = app
        .autocomplete
        .confirmed
        .clone()
        .expect("enabled autocomplete has confirmed policy");
    let context = match app.buffer.cursor_context(
        policy.autocomplete.max_context_before,
        policy.autocomplete.max_context_after,
    ) {
        Ok(context) => context,
        Err(error) => return start_error(app, out, format!("context unavailable: {error}")),
    };
    if !crate::llm::autocomplete::useful_prefix(&context, policy.autocomplete.minimum_prefix_length)
    {
        app.autocomplete.last_edit = None;
        return Ok(());
    }
    let backend = match ConfirmedBackend::resolve(&policy.preset) {
        Ok(backend) if backend.destination() == policy.destination => backend,
        Ok(_) => return confirmation_expired(app, out),
        Err(error) => return start_error(app, out, format!("backend unavailable: {error}")),
    };
    let identity = current_identity(app, &policy);
    let user = crate::llm::autocomplete::user_prompt(&context);
    match LlmTask::start_bounded(
        backend,
        crate::llm::autocomplete::SYSTEM_PROMPT.to_string(),
        user,
        policy.autocomplete.max_generated_tokens,
    ) {
        Ok(task) => {
            app.autocomplete.backoff_until = None;
            app.autocomplete.error = None;
            app.message = None;
            app.autocomplete.running = Some(RunningRequest { task, identity });
            app.render(out)
        }
        Err(error) => start_error(app, out, format!("could not start worker: {error}")),
    }
}

fn finish_result(
    app: &mut super::super::App,
    out: &mut dyn Write,
    identity: RequestIdentity,
    result: LlmTaskResult,
) -> io::Result<()> {
    if !identity_is_current(app, &identity) {
        return Ok(());
    }
    match result {
        LlmTaskResult::Finished(output) => finish_output(app, out, identity, output),
        LlmTaskResult::Cancelled => {
            app.autocomplete.last_edit = None;
            app.render(out)
        }
        LlmTaskResult::Error { kind, message } => {
            app.model_session.record_failure(&identity.preset, kind);
            backoff(app, out, message)
        }
    }
}

fn finish_output(
    app: &mut super::super::App,
    out: &mut dyn Write,
    identity: RequestIdentity,
    output: String,
) -> io::Result<()> {
    let max_tokens = app
        .autocomplete
        .confirmed
        .as_ref()
        .expect("current identity has policy")
        .autocomplete
        .max_generated_tokens;
    match crate::llm::autocomplete::sanitize_output(&output, max_tokens) {
        Ok(text) => {
            app.model_session.record_ready(&identity.preset);
            app.autocomplete.suggestion = Some(Suggestion { text, identity });
            app.autocomplete.last_edit = None;
            app.autocomplete.backoff_until = None;
            app.autocomplete.failures = 0;
            app.autocomplete.error = None;
            app.reveal_cursor();
            app.render(out)
        }
        Err(error) => backoff(app, out, error.to_string()),
    }
}

fn start_error(app: &mut super::super::App, out: &mut dyn Write, error: String) -> io::Result<()> {
    backoff(app, out, error)
}

fn backoff(app: &mut super::super::App, out: &mut dyn Write, error: String) -> io::Result<()> {
    app.autocomplete.failures = app.autocomplete.failures.saturating_add(1);
    let seconds = 1u64
        .checked_shl(u32::from(app.autocomplete.failures.saturating_sub(1)))
        .unwrap_or(30)
        .min(30);
    app.autocomplete.backoff_until = Some(Instant::now() + Duration::from_secs(seconds));
    app.autocomplete.error = Some(error.clone());
    app.message_error(format!(
        "Autocomplete error; retrying after {seconds}s backoff: {error}"
    ));
    app.render(out)
}

pub(super) fn current_identity(
    app: &super::super::App,
    policy: &ConfirmedPolicy,
) -> RequestIdentity {
    RequestIdentity {
        revision: app.buffer.edit_history_position(),
        cursor: app.buffer.cursor(),
        mode: app.mode,
        generation: app.autocomplete.generation,
        preset: policy.preset.name.clone(),
        destination: policy.destination.clone(),
        model: policy.preset.model.clone(),
    }
}

pub(super) fn identity_is_current(app: &super::super::App, identity: &RequestIdentity) -> bool {
    let Some(policy) = app.autocomplete.confirmed.as_ref() else {
        return false;
    };
    app.autocomplete.enabled
        && identity.revision == app.buffer.edit_history_position()
        && identity.cursor == app.buffer.cursor()
        && identity.mode == app.mode
        && identity.generation == app.autocomplete.generation
        && identity.preset == policy.preset.name
        && identity.destination == policy.destination
        && identity.model == policy.preset.model
}

fn confirmation_expired(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    super::invalidate(app);
    app.autocomplete.confirmed = None;
    app.autocomplete.enabled = false;
    app.message_warning(
        "Autocomplete destination changed since confirmation; it is disabled until reconfirmed.",
    );
    app.render(out)
}

#[cfg(test)]
pub(super) fn finish_for_test(
    app: &mut super::super::App,
    out: &mut dyn Write,
    identity: RequestIdentity,
    result: LlmTaskResult,
) -> io::Result<()> {
    finish_result(app, out, identity, result)
}

#[cfg(test)]
pub(super) fn should_start_for_test(app: &super::super::App, now: Instant) -> bool {
    should_start(app, now)
}
