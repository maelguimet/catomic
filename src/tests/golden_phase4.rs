//! Purpose: lock exact source-to-preview output for the Markdown surface.
//! Owns: complete representative Markdown preview golden fixtures.
//! Must not: launch a terminal, touch disk, benchmark, mutate buffers, or network.
//! Invariants: source remains byte-identical; expected preview compares as a whole string.

#[cfg(test)]
mod tests {
    #[test]
    fn golden_markdown_preview_document() {
        let source = "# Heading\n\n- item `code`\n\n> quote";
        let preview = crate::editor::markdown_preview::render_with_width(source, 80).unwrap();

        assert_eq!(preview.text, "Heading\n═══════\n\n• item code\n\n│ quote\n");
        assert_eq!(source, "# Heading\n\n- item `code`\n\n> quote");
    }

    #[test]
    fn golden_markdown_showcase_preview() {
        let source = "# Markdown showcase\n\nNormal **bold**, *italic*, ~~strikethrough~~, `inline code`, and [a link](https://example.com).\n\n| Left | Center | Right |\n| :--- | :----: | ----: |\n| short | `code` | 10 |\n| wide 猫 emoji 🐾 | a much longer value | 2,000 |\n\n> A quote with **formatting**\n\n- [x] complete\n- [ ] incomplete\n\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```";

        let preview = crate::editor::markdown_preview::render_with_width(source, 80).unwrap();

        assert!(preview
            .text
            .starts_with("Markdown showcase\n═════════════════\n\n"));
        assert!(preview
            .text
            .contains("Normal bold, italic, strikethrough, inline code, and a link."));
        assert!(!preview.text.contains("**bold**"));
        assert!(!preview.text.contains("`inline code`"));
        assert!(!preview.text.contains("https://example.com"));
        assert!(preview
            .text
            .contains("│ wide 猫 emoji 🐾 │ a much longer value │ 2,000 │"));
        assert!(preview.text.contains("│ A quote with formatting"));
        assert!(preview.text.contains("• [✓] complete\n• [ ] incomplete"));
        assert!(preview
            .text
            .contains("  fn main() {\n      println!(\"hello\");\n  }"));
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
