//! Purpose: verify representative Markdown preview documents as coherent layouts.
//! Owns: complete fixtures and their block/inline semantic presentation.
//! Must not: launch a terminal, touch disk, benchmark, mutate buffers, or network.
//! Invariants: source remains byte-identical; tests do not bless decorative replacement strings.

#[cfg(test)]
mod tests {
    #[test]
    fn representative_markdown_document_keeps_semantic_layout() {
        let source = "# Heading\n\n- item `code`\n\n> quote";
        let preview = crate::editor::markdown_preview::render_with_width(source, 80).unwrap();

        let lines = preview.text.lines().collect::<Vec<_>>();
        assert_eq!(lines[0].trim(), "Heading");
        assert_eq!(crate::editor::text_layout::cell_width_from(lines[0], 0), 78);
        assert!(preview.text.contains("  • item code"));
        assert!(preview.text.contains("“quote”"));
        assert!(!preview.text.chars().any(|ch| matches!(ch, '#' | '═' | '─')));
        assert!(preview.spans[0]
            .iter()
            .any(|span| { span.style == crate::editor::syntax::SpanStyle::PreviewHeading1 }));
        assert!(preview
            .spans
            .iter()
            .flatten()
            .any(|span| { span.style == crate::editor::syntax::SpanStyle::PreviewInlineCode }));
        assert_eq!(source, "# Heading\n\n- item `code`\n\n> quote");
    }

    #[test]
    fn representative_markdown_showcase_preserves_structure() {
        let source = "# Markdown showcase\n\nNormal **bold**, *italic*, ~~strikethrough~~, `inline code`, and [a link](https://example.com).\n\n| Left | Center | Right |\n| :--- | :----: | ----: |\n| short | `code` | 10 |\n| wide 猫 emoji 🐾 | a much longer value | 2,000 |\n\n> A quote with **formatting**\n\n- [x] complete\n- [ ] incomplete\n\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```";

        let preview = crate::editor::markdown_preview::render_with_width(source, 80).unwrap();

        assert_eq!(
            preview.text.lines().next().map(str::trim),
            Some("Markdown showcase")
        );
        assert!(preview
            .text
            .contains("Normal bold, italic, strikethrough, inline code, and a link."));
        assert!(!preview.text.contains("**bold**"));
        assert!(!preview.text.contains("`inline code`"));
        assert!(!preview.text.contains("https://example.com"));
        assert!(preview.text.contains("wide 猫 emoji 🐾"));
        assert!(preview.text.contains("a much longer value"));
        assert!(preview.text.contains("2,000"));
        assert!(!preview.text.chars().any(|ch| "┌┬┐╞╪╡└┴┘═─".contains(ch)));
        assert!(preview.text.contains("“A quote with formatting”"));
        assert!(preview.text.contains("Left") && preview.text.contains(" │ "));
        assert!(preview
            .text
            .contains("  • [✓] complete\n  • [ ] incomplete"));
        assert!(preview.text.contains("    fn main() {"));
        assert!(preview.text.contains("        println!(\"hello\");"));
        assert!(!preview.text.contains("```"));
        assert!(preview
            .text
            .lines()
            .all(|line| { crate::editor::text_layout::cell_width_from(line, 0) <= 80 }));
        let links = preview.links.iter().flatten().collect::<Vec<_>>();
        assert!(!links.is_empty());
        assert!(links
            .iter()
            .all(|link| link.destination.as_ref() == "https://example.com"));
    }
}
