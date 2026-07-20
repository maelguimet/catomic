//! Purpose: specify complete source-to-preview Markdown behavior.
//! Owns: nested blocks, tables, links, tasks, code, footnotes, HTML, and malformed fixtures.
//! Must not: launch a terminal, touch files, mutate buffers, benchmark, or perform network I/O.
//! Invariants: expected text preserves readable content and table terminal-cell alignment.

use super::*;

#[test]
fn renders_nested_blocks_links_tasks_code_and_footnotes() {
    let source = "## Title\n\n> outer\n> > inner **bold**\n\n- [x] done\n  - child\n\n[link](https://example.com) [^n]\n\n[^n]: note\n\n---\n\n```rs\nlet x = 1;\n```";
    let preview = render_with_width(source, 80).unwrap().text;

    assert!(preview.contains("Title"));
    assert!(preview.contains("“outer”"));
    assert!(preview.contains("“inner bold”"));
    assert!(preview.contains("• [✓] done"));
    assert!(preview.contains("    • child"));
    assert!(preview.contains("link [^n]"));
    assert!(!preview.contains("https://example.com"));
    assert!(preview.contains("[^n] note"));
    assert!(preview.contains("·  ·  ·"));
    assert!(preview.contains("    let x = 1;"));
    assert!(!preview.contains("```"));
    assert_eq!(preview.matches('“').count(), 2);
    assert_eq!(preview.matches('”').count(), 2);
}

#[test]
fn quoted_list_items_keep_their_marker_and_text_on_one_line() {
    let source = "> - quoted item\n>   - nested item";
    let preview = render_with_width(source, 80).unwrap().text;

    assert!(preview.contains("    • quoted item"), "{preview:?}");
    assert!(preview.contains("      • nested item"), "{preview:?}");
    assert!(!preview.contains("• \n"), "{preview:?}");
}

#[test]
fn tables_preserve_alignment_inline_content_escaped_pipes_and_unicode() {
    let source = "| Left | Center | Right |\n| :--- | :----: | ----: |\n| wide 猫 emoji 🐾 | `a\\|b` | 2,000 |\n| é | **longer** | 10 |";
    let preview = render_with_width(source, 80).unwrap();

    assert_eq!(
        preview.text,
        concat!(
            "  Left             │ Center │ Right\n",
            "  wide 猫 emoji 🐾 │  a|b   │ 2,000\n",
            "  é                │ longer │    10\n",
        )
    );
    assert!(!preview.text.chars().any(|ch| "┌┬┐╞╪╡└┴┘═─".contains(ch)));
    assert!(preview.spans[0]
        .iter()
        .any(|span| span.style == SpanStyle::PreviewStrong));
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

    assert!(preview.contains("Name: alpha"));
    assert!(preview.contains("Value: a value"));
    assert!(!preview.contains('┌'));
    assert!(preview
        .lines()
        .all(|line| text_layout::cell_width_from(line, 0) <= 24));
    assert!(preview.contains("  a"));
    assert!(!preview.contains("```"));
}

#[test]
fn stacked_table_records_have_breathing_room() {
    let source = "| Name | Value |\n| --- | ---: |\n| alpha | 1 |\n| beta | 2 |";
    let preview = render_with_width(source, 12).unwrap().text;

    assert!(preview.contains("Value: 1\n\nName: beta"), "{preview:?}");
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
        "“quoted”",
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
    assert!(styles.contains(&SpanStyle::PreviewInlineCode));
    assert!(styles.contains(&SpanStyle::PreviewLink));
    assert_eq!(preview.links.iter().flatten().count(), 1);
}

#[test]
fn document_margin_centers_a_bounded_reading_column() {
    let source = "A deliberately long paragraph that needs a stable reading column instead of stretching across an arbitrarily wide terminal.";
    let preview = render_with_width(source, 120).unwrap();

    let lines = preview.text.lines().collect::<Vec<_>>();
    assert!(lines.len() >= 2);
    assert!(lines.iter().all(|line| line.starts_with(&" ".repeat(16))));
    assert!(lines
        .iter()
        .all(|line| text_layout::cell_width_from(line, 0) <= 104));

    let narrow = render_with_width(source, 24).unwrap();
    assert!(narrow.text.lines().all(|line| !line.starts_with(' ')));
    assert!(narrow
        .text
        .lines()
        .all(|line| text_layout::cell_width_from(line, 0) <= 24));
}

#[test]
fn inline_and_fenced_code_use_distinct_complete_treatments() {
    let source = "Use `inline` here.\n\n```text\none\n\nthree\n```";
    let preview = render_with_width(source, 40).unwrap();
    let code_rows = preview
        .spans
        .iter()
        .enumerate()
        .filter_map(|(row, spans)| {
            spans
                .iter()
                .find(|span| span.style == SpanStyle::PreviewCodeBlock)
                .map(|span| (row, span))
        })
        .collect::<Vec<_>>();

    assert!(preview.spans[0]
        .iter()
        .any(|span| span.style == SpanStyle::PreviewInlineCode));
    assert_eq!(code_rows.len(), 3);
    for (row, span) in code_rows {
        let line = preview.text.lines().nth(row).unwrap();
        assert!(line.starts_with(&" ".repeat(6)));
        assert_eq!(span.start, 2);
        assert_eq!(span.end, line.chars().count());
    }
    assert!(!preview.text.contains('`'));
}

#[test]
fn heading_levels_use_semantic_styles_and_defined_spacing_without_rulers() {
    let source = "# One\n\n## Two\n\n### Three\n\n#### Four\n\n##### Five\n\n###### Six";
    let preview = render_with_width(source, 80).unwrap();

    let lines = preview.text.lines().collect::<Vec<_>>();
    assert_eq!(lines[0].trim(), "One");
    assert_eq!(text_layout::cell_width_from(lines[0], 0), 78);
    assert_eq!(lines[2], "  Two");
    assert_eq!(lines[4], "    Three");
    assert_eq!(lines[5], "      Four");
    assert_eq!(lines[6], "      Five");
    assert_eq!(lines[7], "        Six");
    assert!(!preview.text.contains('#'));
    assert!(!preview.text.chars().any(|ch| matches!(ch, '═' | '─')));
    for (row, style) in [
        (0, SpanStyle::PreviewHeading1),
        (2, SpanStyle::PreviewHeading2),
        (4, SpanStyle::PreviewHeading3),
        (5, SpanStyle::PreviewHeading4),
        (6, SpanStyle::PreviewHeading5),
        (7, SpanStyle::PreviewHeading6),
    ] {
        assert!(preview.spans[row].iter().any(|span| span.style == style));
    }
}

#[test]
fn long_h1_headings_fill_each_wrapped_title_row() {
    let preview = render_with_width(
        "# A deliberately long title that wraps across multiple terminal rows",
        40,
    )
    .unwrap();

    for (line, spans) in preview.text.lines().zip(&preview.spans) {
        assert_eq!(text_layout::cell_width_from(line, 0), 38);
        assert!(spans.iter().any(|span| {
            span.style == SpanStyle::PreviewHeading1 && span.start == 2 && span.end == line.len()
        }));
    }
}

#[test]
fn multiline_links_share_one_safe_destination_across_rendered_lines() {
    let preview = render_with_width("[first  \nsecond](https://example.com)", 80).unwrap();
    let links = preview.links.iter().flatten().collect::<Vec<_>>();

    assert_eq!(preview.text, "  first\n  second\n");
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
