//! Purpose: this file must finish current-buffer LLM tasks against their confirmed identity.
//! Owns: completed-task polling, source/path drift checks, and preview/answer handoff.
//! Must not: construct clients, send requests, apply edits, write files, or collect context.
//! Invariants: changed text or path discards output; edit output still enters read-only preview.
//! Phase: 6 acceptance hardening.

use std::io::{self, Write};

use crate::llm::task::LlmTaskResult;

use super::{RequestPurpose, RunningLlmRequest};

pub(crate) fn poll(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    let result = app
        .llm_task
        .as_mut()
        .and_then(|running| running.task.try_result());
    let Some(result) = result else {
        return Ok(());
    };
    let running = app.llm_task.take().expect("completed task exists");
    match result {
        LlmTaskResult::Finished(output) => finish_output(app, out, output, running),
        LlmTaskResult::Cancelled => render_message(app, out, "LLM request cancelled."),
        LlmTaskResult::Error(error) => {
            render_message(app, out, &format!("LLM request failed: {error}"))
        }
    }
}

fn finish_output(
    app: &mut super::super::App,
    out: &mut dyn Write,
    output: String,
    running: RunningLlmRequest,
) -> io::Result<()> {
    if app.buffer.to_string() != running.source_snapshot {
        return render_message(
            app,
            out,
            "Buffer changed while the model was working; response was not previewed.",
        );
    }
    if app.file.path != running.file_path {
        return render_message(
            app,
            out,
            "Active file path changed while the model was working; response was not previewed.",
        );
    }
    match running.purpose {
        RequestPurpose::Edit => super::super::llm_preview::show_with_region_fallback(
            app,
            out,
            &output,
            Some(&running.path),
            running.replacement_target,
        ),
        RequestPurpose::Explain => super::super::llm_answer::show(app, out, &output),
    }
}

fn render_message(
    app: &mut super::super::App,
    out: &mut dyn Write,
    message: &str,
) -> io::Result<()> {
    app.message = Some(message.to_string());
    app.render(out)
}
