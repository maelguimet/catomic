//! Purpose: this file must open repo-guarded single-file LLM patch previews.
//! Owns: repo proposal parsing and transfer of the broker drift guard into preview state.
//! Must not: apply patches, mutate source, construct clients, read repos, write, or network.
//! Invariants: only valid current-buffer patches reach the read-only guarded preview.
//! Phase: 6 (LLM Context Broker).

use std::io::{self, Write};

use crate::llm::broker::ContextBroker;

pub(crate) fn show_repo_patch(
    app: &mut super::super::App,
    out: &mut dyn Write,
    output: &str,
    guard: ContextBroker,
) -> io::Result<()> {
    let source_snapshot = app.buffer.to_string();
    let (proposal, proposed_text) = match super::proposal::build_patch(&source_snapshot, output) {
        Ok(proposal) => proposal,
        Err(message) => {
            app.message = Some(message);
            return app.render(out);
        }
    };
    super::open(
        app,
        out,
        proposal,
        proposed_text,
        source_snapshot,
        output,
        "Repo LLM patch preview (read-only). Enter rechecks repo and applies; Esc cancels.",
        Some(guard),
    )
}
