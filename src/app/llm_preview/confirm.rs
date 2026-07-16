//! Purpose: this file must apply a preview only against its unchanged source identity.
//! Owns: final repo/path/text rechecks and the one confirmed buffer transaction.
//! Must not: construct clients, send requests, write files, or bypass ordinary undo.
//! Invariants: any repo, active-path, or source-text drift refuses the proposal.
//! Phase: 6 acceptance hardening.

use std::io::{self, Write};

pub(super) fn apply(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    let preview = app.llm_preview.take().expect("preview active");
    app.screen.scroll_top = preview.source_scroll_top;
    app.screen.scroll_left = preview.source_scroll_left;
    if app.file.path != preview.source_path {
        return refuse(
            app,
            out,
            "Active file path changed; LLM proposal was not applied.",
        );
    }
    if let Err(message) = check_repo(&preview) {
        return refuse(app, out, &message);
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
    super::super::input::finish_content_edit_with_message(
        app,
        out,
        Some("LLM proposal applied; Ctrl+Z undoes it.".to_string()),
    )
}

fn check_repo(preview: &super::PatchPreview) -> Result<(), String> {
    let Some(guard) = preview.repo_guard.as_ref() else {
        return Ok(());
    };
    match guard.is_unchanged() {
        Ok(true) => Ok(()),
        Ok(false) => {
            Err("Repository changed since the request; repo LLM patch was not applied.".to_string())
        }
        Err(error) => Err(format!(
            "Could not recheck repository; patch refused: {error}"
        )),
    }
}

fn refuse(app: &mut super::super::App, out: &mut dyn Write, message: &str) -> io::Result<()> {
    app.message = Some(message.to_string());
    app.reveal_cursor();
    app.render(out)
}
