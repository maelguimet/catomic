//! Purpose: this file must finish current-buffer LLM tasks against their confirmed identity.
//! Owns: completed-task polling, source/path drift checks, and preview handoff.
//! Must not: construct clients, send requests, apply edits, write files, or collect context.
//! Invariants: changed text or path discards output; model output enters read-only preview.

use std::io::{self, Write};

use crate::llm::task::LlmTaskResult;

use super::RunningLlmRequest;

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
        LlmTaskResult::Finished(output) => {
            app.model_session.record_ready(&running.preset_name);
            finish_output(app, out, output, running)
        }
        LlmTaskResult::Cancelled => {
            app.message = None;
            app.render(out)
        }
        LlmTaskResult::Error { kind, message } => {
            app.model_session.record_failure(&running.preset_name, kind);
            render_error(app, out, &format!("LLM request failed: {message}"))
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
        return render_warning(
            app,
            out,
            "Buffer changed while the model was working; response was not previewed.",
        );
    }
    if app.file.path != running.file_path {
        return render_warning(
            app,
            out,
            "Active file path changed while the model was working; response was not previewed.",
        );
    }
    super::super::llm_preview::show_with_region_fallback(
        app,
        out,
        &output,
        Some(&running.path),
        running.replacement_target,
    )
}

fn render_warning(
    app: &mut super::super::App,
    out: &mut dyn Write,
    message: &str,
) -> io::Result<()> {
    app.message_warning(message);
    app.render(out)
}

fn render_error(app: &mut super::super::App, out: &mut dyn Write, message: &str) -> io::Result<()> {
    app.message_error(message);
    app.render(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn network_failure_sets_error_role_at_the_emission_boundary() {
        let mut app = super::super::super::App::new(None).unwrap();

        super::render_error(&mut app, &mut Vec::new(), "LLM request failed: boom").unwrap();

        assert_eq!(app.message_role, crate::terminal::render::StatusRole::Error);
    }
}
