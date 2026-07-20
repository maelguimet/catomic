//! Purpose: this file must apply a preview only against its unchanged source identity.
//! Owns: final repo/path/text rechecks and the one confirmed buffer transaction.
//! Must not: construct clients, send requests, write files, or bypass ordinary undo.
//! Invariants: any repo, active-path, or source-text drift refuses the proposal.
//! Phase: 6 acceptance hardening.

use std::io::{self, Write};

pub(super) fn apply(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    if app
        .surfaces
        .llm_preview
        .as_ref()
        .is_some_and(|preview| preview.repo_guard.is_some())
    {
        return begin_repo_check(app, out);
    }
    finish_apply(app, out)
}

fn begin_repo_check(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    if let Some(message) = identity_error(app) {
        super::close(app);
        return refuse(app, out, message);
    }
    let broker = app
        .surfaces
        .llm_preview
        .as_mut()
        .and_then(|preview| preview.repo_guard.take())
        .expect("repo preview has guard");
    if let Err(error) = super::super::repo_llm::begin_apply_check(app, broker) {
        super::close(app);
        return refuse(
            app,
            out,
            &format!("Could not start final repository check; patch refused: {error}"),
        );
    }
    app.message_info("Rechecking repository before apply... Esc cancels.");
    app.render(out)
}

pub(super) fn finish_apply(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    let preview = app.surfaces.llm_preview.take().expect("preview active");
    app.screen.scroll_top = preview.source_scroll_top;
    app.screen.scroll_left = preview.source_scroll_left;
    if app.file.path != preview.source_path {
        return refuse(
            app,
            out,
            "Active file path changed; LLM proposal was not applied.",
        );
    }
    let current = app.buffer.to_string();
    if current != preview.source_snapshot {
        return refuse(
            app,
            out,
            "Source changed since preview; LLM proposal was not applied.",
        );
    }
    if !preview
        .proposal
        .apply(&mut *app.buffer, &current, &preview.proposed_text)?
    {
        return refuse(app, out, "LLM proposal makes no applicable change.");
    }
    super::super::input::finish_content_edit(app, out)
}

fn identity_error(app: &super::super::App) -> Option<&'static str> {
    let preview = app.surfaces.llm_preview.as_ref().expect("preview active");
    if app.file.path != preview.source_path {
        return Some("Active file path changed; LLM proposal was not applied.");
    }
    (app.buffer.to_string() != preview.source_snapshot)
        .then_some("Source changed since preview; LLM proposal was not applied.")
}

fn refuse(app: &mut super::super::App, out: &mut dyn Write, message: &str) -> io::Result<()> {
    app.message_warning(message);
    app.reveal_cursor();
    app.render(out)
}
