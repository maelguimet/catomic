//! Purpose: specify complete source-to-preview Markdown behavior.
//! Owns: nested blocks, tables, links, tasks, code, footnotes, HTML, and malformed fixtures.
//! Must not: launch a terminal, touch files, mutate buffers, benchmark, or perform network I/O.
//! Invariants: expected text preserves readable content and table terminal-cell alignment.
//! Phase: issue #54 Markdown preview regression coverage.

use super::*;

#[test]
fn renders_nested_blocks_links_tasks_code_and_footnotes() {
    let source = "## Title\n\n> outer\n> > inner **bold**\n\n- [x] done\n  - child\n\n[link](https://example.com) [^n]\n\n[^n]: note\n\n---\n\n```rs\nlet x = 1;\n```";
    let preview = render(source);

    assert!(preview.contains("▌▌ Title"));
    assert!(preview.contains("│ │ inner **bold**"));
    assert!(preview.contains("• [x] done"));
    assert!(preview.contains("  • child"));
    assert!(preview.contains("link ⟨https://example.com⟩ [^n]"));
    assert!(preview.contains("[^n] note"));
    assert!(preview.contains("────────────────────────"));
    assert!(preview.contains("┌─ code: rs\n┊ let x = 1;\n└─"));
}

#[test]
fn tables_preserve_alignment_inline_content_escaped_pipes_and_unicode() {
    let source = "| Left | Center | Right |\n| :--- | :----: | ----: |\n| wide 猫 emoji 🐾 | `a\\|b` | 2,000 |\n| é | **longer** | 10 |";
    let preview = render(source);

    assert_eq!(
        preview,
        "┌──────────────────┬────────────┬───────┐\n\
         │ Left             │   Center   │ Right │\n\
         ╞══════════════════╪════════════╪═══════╡\n\
         │ wide 猫 emoji 🐾 │   ‹a|b›    │ 2,000 │\n\
         │ é                │ **longer** │    10 │\n\
         └──────────────────┴────────────┴───────┘\n"
    );
}

#[test]
fn raw_html_and_malformed_markdown_remain_inert_readable_text() {
    let source = "<script>escape\u{1b}[2J</script>\n\n[broken](url\n\n| malformed | row |";
    let preview = render(source);

    assert!(preview.contains("<script>escape\u{1b}[2J</script>"));
    assert!(preview.contains("[broken](url"));
    assert!(preview.contains("| malformed | row |"));
}
