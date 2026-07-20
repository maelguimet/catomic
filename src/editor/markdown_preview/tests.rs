//! Purpose: specify complete source-to-preview Markdown behavior.
//! Owns: nested blocks, tables, links, tasks, code, footnotes, HTML, and malformed fixtures.
//! Must not: launch a terminal, touch files, mutate buffers, benchmark, or perform network I/O.
//! Invariants: expected text preserves readable content and table terminal-cell alignment.
//! Phase: issue #54 Markdown preview regression coverage.

use super::*;

#[test]
fn renders_nested_blocks_links_tasks_code_and_footnotes() {
    let source = "## Title\n\n> outer\n> > inner **bold**\n\n- [x] done\n  - child\n\n[link](https://example.com) [^n]\n\n[^n]: note\n\n---\n\n```rs\nlet x = 1;\n```";
    let preview = render_with_width(source, 80).unwrap();

    assert!(preview.contains("## Title"));
    assert!(preview.contains("> > inner **bold**"));
    assert!(preview.contains("- [x] done"));
    assert!(preview.contains("  - child"));
    assert!(preview.contains("link <https://example.com> [^n]"));
    assert!(preview.contains("[^n] note"));
    assert!(preview.contains("────────────────────────"));
    assert!(preview.contains("```rs\n    let x = 1;\n```"));
}

#[test]
fn tables_preserve_alignment_inline_content_escaped_pipes_and_unicode() {
    let source = "| Left | Center | Right |\n| :--- | :----: | ----: |\n| wide 猫 emoji 🐾 | `a\\|b` | 2,000 |\n| é | **longer** | 10 |";
    let preview = render_with_width(source, 80).unwrap();

    assert_eq!(
        preview,
        "┌──────────────────┬────────────┬───────┐\n\
         │ Left             │   Center   │ Right │\n\
         ╞══════════════════╪════════════╪═══════╡\n\
         │ wide 猫 emoji 🐾 │   `a|b`    │ 2,000 │\n\
         │ é                │ **longer** │    10 │\n\
         └──────────────────┴────────────┴───────┘\n"
    );
}

#[test]
fn raw_html_and_malformed_markdown_remain_inert_readable_text() {
    let source = "<script>escape\u{1b}[2J</script>\n\n[broken](url\n\n| malformed | row |";
    let preview = render_with_width(source, 80).unwrap();

    assert!(preview.contains("<script>escape␛[2J</script>"));
    assert!(preview.contains("[broken](url"));
    assert!(preview.contains("| malformed | row |"));
}

#[test]
fn narrow_layout_wraps_every_line_and_stacks_tables() {
    let source = "# A deliberately long heading for a narrow terminal\n\nA paragraph with Unicode 猫🐾, tabs\tand https://example.com/a/very/long/path/that/cannot/fit.\n\n| Name | Value |\n| --- | ---: |\n| alpha | a value that is much too wide |\n\n```text\na\tvery long code line that must wrap safely\n```";
    let preview = render_with_width(source, 24).unwrap();

    assert!(preview.contains("- Name: alpha"));
    assert!(preview.contains("- Value: a value"));
    assert!(!preview.contains('┌'));
    assert!(preview
        .lines()
        .all(|line| text_layout::cell_width_from(line, 0) <= 24));
    assert!(preview.contains("```text"));
}

#[test]
fn source_like_markers_keep_monochrome_output_readable() {
    let source = "# Heading\n\nParagraph with **strong**, *emphasis*, ~~strike~~, and `code`.\n\n1. first\n2. second\n\n> quoted\n\n![alt](image.png)";
    let preview = render_with_width(source, 80).unwrap();

    for expected in [
        "# Heading",
        "**strong**",
        "*emphasis*",
        "~~strike~~",
        "`code`",
        "1. first",
        "> quoted",
        "Image: alt <image.png>",
    ] {
        assert!(
            preview.contains(expected),
            "missing {expected:?}: {preview}"
        );
    }
    assert!(!preview.contains('▌'));
    assert!(!preview.contains('‹'));
}

#[test]
fn pathological_table_shape_returns_a_real_render_error() {
    let header = (0..=MAX_TABLE_COLUMNS_FOR_FIXTURE)
        .map(|column| format!(" c{column} "))
        .collect::<Vec<_>>()
        .join("|");
    let separator = std::iter::repeat_n(" --- ", MAX_TABLE_COLUMNS_FOR_FIXTURE + 1)
        .collect::<Vec<_>>()
        .join("|");
    let source = format!("|{header}|\n|{separator}|\n");

    assert_eq!(
        render_with_width(&source, 80),
        Err(RenderError::TableComplexity)
    );
}

const MAX_TABLE_COLUMNS_FOR_FIXTURE: usize = 128;
