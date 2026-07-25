//! Purpose: this file must apply a preview only against its unchanged source identity.
//! Owns: final path/text rechecks and the one confirmed buffer transaction.
//! Must not: construct clients, send requests, write files, or bypass ordinary undo.
//! Invariants: any active-path or source-text drift refuses the proposal.

use std::io::{self, Write};

pub(super) fn apply(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    finish_apply(app, out)
}

fn finish_apply(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
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

fn refuse(app: &mut super::super::App, out: &mut dyn Write, message: &str) -> io::Result<()> {
    app.message_warning(message);
    app.reveal_cursor();
    app.render(out)
}
