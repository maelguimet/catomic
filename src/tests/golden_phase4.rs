//! Purpose: lock exact source-to-preview output for the Phase 4 Markdown surface.
//! Owns: complete representative Markdown preview golden fixtures.
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

    #[test]
    fn golden_markdown_showcase_preview() {
        let source = "# Markdown showcase\n\nNormal **bold**, *italic*, ~~strikethrough~~, `inline code`, and [a link](https://example.com).\n\n| Left | Center | Right |\n| :--- | :----: | ----: |\n| short | `code` | 10 |\n| wide 猫 emoji 🐾 | a much longer value | 2,000 |\n\n> A quote with **formatting**\n\n- [x] complete\n- [ ] incomplete\n\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```";

        assert_eq!(
            crate::editor::markdown_preview::render(source),
            "▌ Markdown showcase\n\n\
             Normal **bold**, *italic*, ~~strikethrough~~, ‹inline code›, and a link ⟨https://example.com⟩.\n\n\
             ┌──────────────────┬─────────────────────┬───────┐\n\
             │ Left             │       Center        │ Right │\n\
             ╞══════════════════╪═════════════════════╪═══════╡\n\
             │ short            │       ‹code›        │    10 │\n\
             │ wide 猫 emoji 🐾 │ a much longer value │ 2,000 │\n\
             └──────────────────┴─────────────────────┴───────┘\n\n\
             │ A quote with **formatting**\n\n\
             • [x] complete\n\
             • [ ] incomplete\n\n\
             ┌─ code: rust\n\
             ┊ fn main() {\n\
             ┊     println!(\"hello\");\n\
             ┊ }\n\
             └─\n"
        );
    }
}
