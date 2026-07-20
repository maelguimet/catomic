//! Purpose: specify complete source-to-preview Markdown behavior.
//! Owns: nested blocks, tables, links, tasks, code, footnotes, HTML, and malformed fixtures.
//! Must not: launch a terminal, touch files, mutate buffers, benchmark, or perform network I/O.
//! Invariants: expected text preserves readable content and table terminal-cell alignment.
//! Phase: issue #54 Markdown preview regression coverage.

use super::*;

#[test]
fn renders_nested_blocks_links_tasks_code_and_footnotes() {
    let source = "## Title\n\n> outer\n> > inner **bold**\n\n- [x] done\n  - child\n\n[link](https://example.com) [^n]\n\n[^n]: note\n\n---\n\n```rs\nlet x = 1;\n```";
    let preview = render_with_width(source, 80).unwrap().text;

    assert!(preview.contains("Title"));
    assert!(preview.contains("│ │ inner bold"));
    assert!(preview.contains("• [✓] done"));
    assert!(preview.contains("  • child"));
    assert!(preview.contains("link [^n]"));
    assert!(!preview.contains("https://example.com"));
    assert!(preview.contains("[^n] note"));
    assert!(preview.contains("────────────────────────"));
    assert!(preview.contains("  let x = 1;"));
    assert!(!preview.contains("```"));
}

#[test]
fn tables_preserve_alignment_inline_content_escaped_pipes_and_unicode() {
    let source = "| Left | Center | Right |\n| :--- | :----: | ----: |\n| wide 猫 emoji 🐾 | `a\\|b` | 2,000 |\n| é | **longer** | 10 |";
    let preview = render_with_width(source, 80).unwrap();

    assert!(preview
        .spans
        .iter()
        .flatten()
        .any(|span| span.style == SpanStyle::Marker));

    assert_eq!(
        preview.text,
        "┌──────────────────┬────────┬───────┐\n\
         │ Left             │ Center │ Right │\n\
         ╞══════════════════╪════════╪═══════╡\n\
         │ wide 猫 emoji 🐾 │  a|b   │ 2,000 │\n\
         │ é                │ longer │    10 │\n\
         └──────────────────┴────────┴───────┘\n"
    );
}

#[test]
fn raw_html_and_malformed_markdown_remain_inert_readable_text() {
    let source = "<script>escape\u{1b}[2J</script>\n\n[broken](url\n\n| malformed | row |";
    let preview = render_with_width(source, 80).unwrap().text;

    assert!(preview.contains("<script>escape␛[2J</script>"));
    assert!(preview.contains("[broken](url"));
    assert!(preview.contains("| malformed | row |"));
}

#[test]
fn narrow_layout_wraps_every_line_and_stacks_tables() {
    let source = "# A deliberately long heading for a narrow terminal\n\nA paragraph with Unicode 猫🐾, tabs\tand https://example.com/a/very/long/path/that/cannot/fit.\n\n| Name | Value |\n| --- | ---: |\n| alpha | a value that is much too wide |\n\n```text\na\tvery long code line that must wrap safely\n```";
    let preview = render_with_width(source, 24).unwrap().text;

    assert!(preview.contains("- Name: alpha"));
    assert!(preview.contains("- Value: a value"));
    assert!(!preview.contains('┌'));
    assert!(preview
        .lines()
        .all(|line| text_layout::cell_width_from(line, 0) <= 24));
    assert!(preview.contains("  a"));
    assert!(!preview.contains("```"));
}

#[test]
fn semantic_output_does_not_regenerate_source_delimiters() {
    let source = "# Heading\n\nParagraph with **strong**, *emphasis*, ~~strike~~, and `code`.\n\n1. first\n2. second\n\n> quoted\n\n![alt](image.png)";
    let preview = render_with_width(source, 80).unwrap();

    for expected in [
        "Heading",
        "strong",
        "emphasis",
        "strike",
        "code",
        "1. first",
        "│ quoted",
        "Image: alt",
    ] {
        assert!(
            preview.text.contains(expected),
            "missing {expected:?}: {}",
            preview.text
        );
    }
    for delimiter in [
        "# Heading",
        "**strong**",
        "*emphasis*",
        "~~strike~~",
        "`code`",
    ] {
        assert!(!preview.text.contains(delimiter), "{}", preview.text);
    }
    let styles = preview
        .spans
        .iter()
        .flatten()
        .map(|span| span.style)
        .collect::<Vec<_>>();
    assert!(styles.contains(&SpanStyle::PreviewStrong));
    assert!(styles.contains(&SpanStyle::PreviewEmphasis));
    assert!(styles.contains(&SpanStyle::PreviewStrikethrough));
    assert!(styles.contains(&SpanStyle::PreviewCode));
    assert!(styles.contains(&SpanStyle::PreviewLink));
    assert_eq!(preview.links.iter().flatten().count(), 1);
}

#[test]
fn heading_levels_keep_a_restrained_terminal_hierarchy() {
    let source = "# One\n\n## Two\n\n### Three\n\n#### Four\n\n##### Five\n\n###### Six";
    let preview = render_with_width(source, 80).unwrap();

    assert_eq!(
        preview.text,
        "One\n═══\n\nTwo\n───\n\nThree\n\nFour\n\nFive\n\nSix\n"
    );
    assert!(!preview.text.contains('#'));
    let styles = preview
        .spans
        .iter()
        .flatten()
        .map(|span| span.style)
        .collect::<Vec<_>>();
    assert!(styles.contains(&SpanStyle::Heading));
    assert!(styles.contains(&SpanStyle::PreviewHeading4));
    assert!(styles.contains(&SpanStyle::PreviewHeading5));
    assert!(styles.contains(&SpanStyle::PreviewHeading6));
}

#[test]
fn multiline_links_share_one_safe_destination_across_rendered_lines() {
    let preview = render_with_width("[first  \nsecond](https://example.com)", 80).unwrap();
    let links = preview.links.iter().flatten().collect::<Vec<_>>();

    assert_eq!(preview.text, "first\nsecond\n");
    assert_eq!(links.len(), 2);
    assert!(Arc::ptr_eq(&links[0].destination, &links[1].destination));
    assert!(links
        .iter()
        .all(|link| link.destination.as_ref() == "https://example.com"));
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
