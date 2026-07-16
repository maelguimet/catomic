//! Purpose: lock exact source-to-preview output for the Phase 4 Markdown surface.
//! Owns: one complete representative Markdown preview golden fixture.
//! Must not: launch a terminal, touch disk, benchmark, mutate buffers, or network.
//! Invariants: source remains byte-identical; expected preview compares as a whole string.
//! Phase: 4 acceptance golden coverage.

#[cfg(test)]
mod tests {
    #[test]
    fn golden_markdown_preview_document() {
        let source = "# Heading\n\n- item `code`\n\n> quote";
        let preview = crate::editor::markdown_preview::render(source);

        assert_eq!(preview, "▌ Heading\n\n• item ‹code›\n\n│ quote\n");
        assert_eq!(source, "# Heading\n\n- item `code`\n\n> quote");
    }
}
