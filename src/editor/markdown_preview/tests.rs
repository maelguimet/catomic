//! Purpose: specify complete source-to-preview Markdown behavior.
//! Owns: nested blocks, tables, links, tasks, code, footnotes, HTML, and malformed fixtures.
//! Must not: launch a terminal, touch files, mutate buffers, benchmark, or perform network I/O.
//! Invariants: expected text preserves readable content and table terminal-cell alignment.

use super::*;
use crate::buffer::Buffer;

fn row_spans(preview: &MarkdownDocument, row: usize) -> Vec<StyledSpan> {
    preview.annotations.spans(row).collect()
}

fn all_spans(preview: &MarkdownDocument) -> Vec<StyledSpan> {
    (0..preview.text.lines().count())
        .flat_map(|row| preview.annotations.spans(row))
        .collect()
}

fn all_links(preview: &MarkdownDocument) -> Vec<HyperlinkSpan> {
    (0..preview.text.lines().count())
        .flat_map(|row| preview.annotations.links(row))
        .collect()
}

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
    assert!(row_spans(&preview, 0)
        .into_iter()
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
    let styles = all_spans(&preview)
        .into_iter()
        .map(|span| span.style)
        .collect::<Vec<_>>();
    assert!(styles.contains(&SpanStyle::PreviewStrong));
    assert!(styles.contains(&SpanStyle::PreviewEmphasis));
    assert!(styles.contains(&SpanStyle::PreviewStrikethrough));
    assert!(styles.contains(&SpanStyle::PreviewInlineCode));
    assert!(styles.contains(&SpanStyle::PreviewLink));
    assert_eq!(all_links(&preview).len(), 1);
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
        .text
        .lines()
        .enumerate()
        .filter_map(|(row, _)| {
            row_spans(&preview, row)
                .into_iter()
                .find(|span| span.style == SpanStyle::PreviewCodeBlock)
                .map(|span| (row, span))
        })
        .collect::<Vec<_>>();

    assert!(row_spans(&preview, 0)
        .into_iter()
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
        assert!(row_spans(&preview, row)
            .into_iter()
            .any(|span| span.style == style));
    }
}

#[test]
fn long_h1_headings_fill_each_wrapped_title_row() {
    let preview = render_with_width(
        "# A deliberately long title that wraps across multiple terminal rows",
        40,
    )
    .unwrap();

    for (row, line) in preview.text.lines().enumerate() {
        let spans = row_spans(&preview, row);
        assert_eq!(text_layout::cell_width_from(line, 0), 38);
        assert!(spans.iter().any(|span| {
            span.style == SpanStyle::PreviewHeading1 && span.start == 2 && span.end == line.len()
        }));
    }
}

#[test]
fn multiline_links_share_one_safe_destination_across_rendered_lines() {
    let preview = render_with_width("[first  \nsecond](https://example.com)", 80).unwrap();
    let links = all_links(&preview);

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

#[test]
fn newline_dense_plain_preview_keeps_only_compact_boundaries() {
    const ROWS: usize = 32 * 1024;
    let source = "x  \n".repeat(ROWS);
    let preview = render_with_width(&source, 80).unwrap();
    let rendered_rows = preview.text.lines().count();

    assert_eq!(rendered_rows, ROWS);
    assert_eq!(preview.line_starts.len(), ROWS + 1);
    assert_eq!(preview.annotations.annotated_row_count(), 0);
    assert_eq!(preview.annotations.annotation_count(), 0);
    assert_eq!(preview.annotations.retained_bytes(), 0);
    assert!(
        preview.text.capacity() < source.len().saturating_mul(2),
        "a trailing synthetic row must not double the output allocation",
    );
    assert!(
        preview.retained_bytes()
            <= preview
                .text
                .capacity()
                .saturating_add((ROWS + 1).saturating_mul(8)),
        "retained={} text={} rows={rendered_rows}",
        preview.retained_bytes(),
        preview.text.capacity(),
    );
}

#[test]
#[cfg(target_pointer_width = "64")]
fn compact_annotations_promote_losslessly_at_row_and_column_boundaries() {
    let mut builder = AnnotationBuilder::default();
    let shared_destination: Arc<str> = Arc::from("https://shared.example");
    builder
        .push_row(
            7,
            vec![StyledSpan {
                start: 1,
                end: 2,
                style: SpanStyle::PreviewStrong,
            }],
            vec![HyperlinkSpan {
                start: 3,
                end: 4,
                destination: Arc::clone(&shared_destination),
            }],
        )
        .unwrap();
    builder
        .push_row(
            10,
            vec![StyledSpan {
                start: 5,
                end: 6,
                style: SpanStyle::PreviewInlineCode,
            }],
            vec![HyperlinkSpan {
                start: 7,
                end: 8,
                destination: Arc::from("https://compact.example"),
            }],
        )
        .unwrap();
    let overflow = u32::MAX as usize + 1;
    builder
        .push_row(
            overflow,
            vec![StyledSpan {
                start: overflow,
                end: overflow + 1,
                style: SpanStyle::PreviewEmphasis,
            }],
            vec![HyperlinkSpan {
                start: overflow + 2,
                end: overflow + 3,
                destination: Arc::clone(&shared_destination),
            }],
        )
        .unwrap();
    builder
        .push_row(
            overflow + 5,
            vec![StyledSpan {
                start: 9,
                end: 11,
                style: SpanStyle::PreviewCodeBlock,
            }],
            vec![HyperlinkSpan {
                start: 12,
                end: 14,
                destination: Arc::from("https://wide.example"),
            }],
        )
        .unwrap();
    let annotations = builder.finish();

    assert_eq!(
        annotations.spans(7).collect::<Vec<_>>(),
        vec![StyledSpan {
            start: 1,
            end: 2,
            style: SpanStyle::PreviewStrong,
        }]
    );
    assert_eq!(
        annotations.spans(10).collect::<Vec<_>>(),
        vec![StyledSpan {
            start: 5,
            end: 6,
            style: SpanStyle::PreviewInlineCode,
        }]
    );
    assert_eq!(
        annotations.spans(overflow).collect::<Vec<_>>(),
        vec![StyledSpan {
            start: overflow,
            end: overflow + 1,
            style: SpanStyle::PreviewEmphasis,
        }]
    );
    assert_eq!(
        annotations.spans(overflow + 5).collect::<Vec<_>>(),
        vec![StyledSpan {
            start: 9,
            end: 11,
            style: SpanStyle::PreviewCodeBlock,
        }]
    );
    assert_eq!(annotations.spans(8).count(), 0);
    assert_eq!(annotations.links(8).count(), 0);
    let compact_link = annotations.links(7).next().unwrap();
    let link = annotations.links(overflow).next().unwrap();
    assert_eq!((link.start, link.end), (overflow + 2, overflow + 3));
    assert!(Arc::ptr_eq(&compact_link.destination, &shared_destination));
    assert!(Arc::ptr_eq(&link.destination, &shared_destination));
    assert_eq!(
        annotations
            .links(overflow + 5)
            .next()
            .unwrap()
            .destination
            .as_ref(),
        "https://wide.example"
    );
}

#[test]
fn sparse_annotation_lookup_handles_large_unannotated_row_gaps() {
    let mut builder = AnnotationBuilder::default();
    for row in [1, 10_000, 1_000_000] {
        builder
            .push_row(
                row,
                vec![StyledSpan {
                    start: row % 7,
                    end: row % 7 + 1,
                    style: SpanStyle::PreviewStrong,
                }],
                Vec::new(),
            )
            .unwrap();
    }
    let annotations = builder.finish();

    assert_eq!(annotations.annotated_row_count(), 3);
    assert_eq!(annotations.spans(999_999).count(), 0);
    assert_eq!(annotations.spans(1_000_000).count(), 1);
}

#[test]
fn retained_bytes_count_unique_link_allocations_once() {
    let shared: Arc<str> = Arc::from("https://shared.example/path");
    let distinct: Arc<str> = Arc::from("https://distinct.example/path");
    let mut builder = AnnotationBuilder::default();
    for (row, destination) in [
        (1, Arc::clone(&shared)),
        (2, Arc::clone(&shared)),
        (10_000, Arc::clone(&distinct)),
    ] {
        builder
            .push_row(
                row,
                Vec::new(),
                vec![HyperlinkSpan {
                    start: 0,
                    end: 1,
                    destination,
                }],
            )
            .unwrap();
    }
    let annotations = builder.finish();
    let expected_destinations =
        arc_str_allocation_bytes(&shared) + arc_str_allocation_bytes(&distinct);

    assert_eq!(
        annotations.link_destination_retained_bytes(),
        expected_destinations
    );
    assert_eq!(
        annotations.retained_bytes(),
        annotations
            .container_retained_bytes()
            .saturating_add(expected_destinations)
    );

    let document = MarkdownDocument {
        text: "x\n".to_string(),
        line_starts: {
            let mut starts = CompactLineStarts::new();
            starts.push(2);
            starts
        },
        annotations,
    };
    assert_eq!(
        document.retained_bytes(),
        document
            .text
            .capacity()
            .saturating_add(document.line_starts.retained_bytes())
            .saturating_add(document.annotations.retained_bytes())
    );
}

#[test]
fn newline_dense_preview_preserves_a_borrowed_terminal_empty_row() {
    let preview = render_with_width("a  \nb  \n", 80).unwrap();
    let (buffer, annotations) = preview.into_buffer_and_annotations();

    assert_eq!(buffer.to_string(), "  a\n  b\n");
    assert_eq!(buffer.line_count(), 3);
    assert!(matches!(buffer.line(2), Some(Cow::Borrowed(""))));
    assert!(matches!(
        buffer.visible_lines_window(2, 1, 0, 80)[0].content,
        Cow::Borrowed("")
    ));
    assert_eq!(annotations.annotated_row_count(), 0);
}

const MAX_TABLE_COLUMNS_FOR_FIXTURE: usize = 128;
