//! Purpose: start and poll exactly one confirmed inline-clanker request at a time.
//! Owns: lazy API-key/client construction, serial progress, response handoff, and stop policy.
//! Must not: discover scope, apply proposals, save files, or start queued work concurrently.
//! Invariants: identity/guards match before every send and response preview; one task exists at most.
//! Phase: issue #65 one-key inline clanker workflow.

use std::io::{self, Write};

use crate::llm::backend::ConfirmedBackend;
use crate::llm::inline::InlineScope;
use crate::llm::task::{LlmTask, LlmTaskResult};

use super::{Phase, PreparedWorkflow, RunningRequest};

const REGION_SYSTEM_PROMPT: &str = "You edit one exact Catomic region. Return exactly one JSON object with one string field named catomic_replacement. Do not return a patch, markdown fence, extra field, or prose. The replacement is only for the supplied region; never edit outside it or claim it was applied.";
const MULTI_SYSTEM_PROMPT: &str = "You edit numbered, independent Catomic context blocks. Return exactly one JSON object with one field catomic_replacements, an array containing every block exactly once as {\"block\":NUMBER,\"replacement\":STRING}. Do not merge concepts, move content between blocks, add prose, or mention content outside the supplied boundaries.";
const FULL_FILE_SYSTEM_PROMPT: &str = "You edit one bounded Catomic file. Return exactly one valid single-file unified diff against the supplied path, without prose or fences. The CATOMIC-INSTRUCTION-METADATA sentinel is immutable control metadata: preserve it byte-identically and exactly once. Never claim the patch was applied.";

pub(super) fn confirm(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    let Some(Phase::Confirm(confirmation)) = app.inline_clanker.phase.take() else {
        return Ok(());
    };
    super::confirmation::restore_view(app, &confirmation);
    let prepared = confirmation.prepared;
    if let Err(message) = validate_identity(app, &prepared) {
        app.message = Some(format!(
            "Inline clanker cancelled before sending: {message}; no network call made."
        ));
        return app.render(out);
    }
    start(app, out, prepared)
}

pub(super) fn poll(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    let result = match app.inline_clanker.phase.as_mut() {
        Some(Phase::Running(running)) => running.task.try_result(),
        _ => None,
    };
    let Some(result) = result else {
        return Ok(());
    };
    let Some(Phase::Running(running)) = app.inline_clanker.phase.take() else {
        unreachable!("completed inline task must still be running")
    };
    match result {
        LlmTaskResult::Finished(output) => {
            app.model_session
                .record_ready(&running.prepared.preset.name);
            finish_output(app, out, running.prepared, output)
        }
        LlmTaskResult::Cancelled => {
            fail_or_continue(app, out, running.prepared, "request cancelled")
        }
        LlmTaskResult::Error { kind, message } => {
            app.model_session
                .record_failure(&running.prepared.preset.name, kind);
            fail_or_continue(app, out, running.prepared, &message)
        }
    }
}

pub(super) fn start(
    app: &mut super::super::App,
    out: &mut dyn Write,
    prepared: PreparedWorkflow,
) -> io::Result<()> {
    if let Err(message) = validate_identity(app, &prepared) {
        app.message = Some(format!(
            "Inline clanker queue stopped before send: {message}."
        ));
        return app.render(out);
    }
    let backend = match ConfirmedBackend::resolve(&prepared.preset) {
        Ok(backend) => backend,
        Err(error) => {
            app.model_session
                .record_failure(&prepared.preset.name, error.kind);
            return fail_or_continue(
                app,
                out,
                prepared,
                &format!("could not prepare configured backend: {error}"),
            );
        }
    };
    if backend.destination() != prepared.destination {
        app.model_session.record_failure(
            &prepared.preset.name,
            crate::llm::backend::BackendErrorKind::Unavailable,
        );
        return fail_or_continue(
            app,
            out,
            prepared,
            "configured backend identity changed after confirmation",
        );
    }
    let unit = &prepared.draft.requests[prepared.request_index];
    let system = system_prompt(&prepared, unit.target_ids.len()).to_string();
    let user = user_prompt(&prepared);
    match LlmTask::start(backend, system, user) {
        Ok(task) => {
            app.message = Some(progress_message(&prepared));
            app.inline_clanker.phase = Some(Phase::Running(RunningRequest { prepared, task }));
        }
        Err(error) => {
            return fail_or_continue(
                app,
                out,
                prepared,
                &format!("could not start request: {error}"),
            )
        }
    }
    app.render(out)
}

fn finish_output(
    app: &mut super::super::App,
    out: &mut dyn Write,
    prepared: PreparedWorkflow,
    output: String,
) -> io::Result<()> {
    if let Err(message) = validate_identity(app, &prepared) {
        app.message = Some(format!(
            "Inline clanker response discarded because {message}; instruction kept."
        ));
        return app.render(out);
    }
    super::preview::open_response(app, out, prepared, &output)
}

pub(super) fn fail_or_continue(
    app: &mut super::super::App,
    out: &mut dyn Write,
    mut prepared: PreparedWorkflow,
    error: &str,
) -> io::Result<()> {
    prepared.had_failure = true;
    let failed = prepared.request_index + 1;
    let has_more = failed < prepared.draft.requests.len();
    if prepared.inline.stop_on_error || !has_more {
        app.message = Some(format!(
            "Inline clanker stopped at request {failed}/{}: {error}; instruction kept and no pending work remains.",
            prepared.draft.requests.len()
        ));
        return app.render(out);
    }
    prepared.request_index += 1;
    app.message = Some(format!(
        "Inline clanker request {failed} failed ({error}); continuing by configuration."
    ));
    start(app, out, prepared)
}

pub(super) fn validate_identity(
    app: &super::super::App,
    prepared: &PreparedWorkflow,
) -> Result<(), &'static str> {
    if app.file.path != prepared.file_path {
        return Err("the active file path changed");
    }
    if app.buffer.edit_history_position() != prepared.expected_revision {
        return Err("the file revision changed");
    }
    if !captured_matches(app, &prepared.draft.instruction.metadata)
        || !captured_matches(app, &prepared.draft.instruction.cleanup)
    {
        return Err("the selected instruction metadata drifted");
    }
    if prepared
        .draft
        .delimiter_guards
        .iter()
        .any(|guard| !captured_matches(app, guard))
    {
        return Err("a context delimiter drifted");
    }
    if prepared
        .draft
        .targets
        .iter()
        .any(|target| !captured_matches(app, &target.range))
    {
        return Err("a captured edit target drifted");
    }
    Ok(())
}

fn captured_matches(app: &super::super::App, captured: &crate::llm::inline::CapturedRange) -> bool {
    app.buffer
        .text_range(captured.start, captured.end)
        .is_ok_and(|text| text == captured.original)
}

fn system_prompt(prepared: &PreparedWorkflow, target_count: usize) -> &'static str {
    match prepared.draft.scope {
        InlineScope::FullFile => FULL_FILE_SYSTEM_PROMPT,
        InlineScope::Blocks if target_count > 1 => MULTI_SYSTEM_PROMPT,
        InlineScope::Selection | InlineScope::Blocks => REGION_SYSTEM_PROMPT,
    }
}

fn user_prompt(prepared: &PreparedWorkflow) -> String {
    let unit = &prepared.draft.requests[prepared.request_index];
    let ranges = if unit.target_ids.is_empty() {
        "full file".to_string()
    } else {
        unit.target_ids
            .iter()
            .filter_map(|&id| prepared.draft.target(id))
            .map(|target| {
                format!(
                    "block {}=lines {}-{} ({} bytes)",
                    target.id,
                    target.range.first_line + 1,
                    target.range.last_line + 1,
                    target.range.original.len()
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "Path: {}\nInstruction source line: {}\nInstruction cleanup after final apply: {}\nConfirmed target: {}\nInstruction:\n{}\n\nContext:\n{}",
        prepared.path,
        prepared.draft.instruction.display_line,
        prepared.inline.remove_instruction_after_apply,
        ranges,
        prepared.draft.instruction.text,
        unit.text
    )
}

fn progress_message(prepared: &PreparedWorkflow) -> String {
    let current = prepared.request_index + 1;
    let total = prepared.draft.requests.len();
    let remaining = total.saturating_sub(current);
    let unit = &prepared.draft.requests[prepared.request_index];
    if total > 1 {
        format!(
            "Inline clanker block {current}/{total}: sending {} lines/{} bytes to {} at {}; {remaining} remaining. Esc cancels queue.",
            unit.line_count,
            unit.byte_count,
            prepared.preset.model,
            prepared.destination
        )
    } else {
        format!(
            "Inline clanker sending {} lines/{} bytes to {} at {}. Esc cancels.",
            unit.line_count, unit.byte_count, prepared.preset.model, prepared.destination
        )
    }
}
