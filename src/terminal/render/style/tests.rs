//! Purpose: verify ANSI composition for syntax and active document ranges.
//! Owns: exact visible-line color, semantic highlights, and scalar-offset fixtures.
//! Must not: query buffers, require a terminal, inspect files, or test syntax detection.
//! Invariants: transitions are byte-exact and rows/hyperlinks end in a safe default state.

use super::*;
use crate::buffer::Cursor;
use crate::editor::syntax::SyntaxKind;
use crate::tests::perf::count_thread_allocations;

fn rendered(content: &str, start_col: usize, options: RenderOptions) -> String {
    rendered_row(content, 0, start_col, options)
}

fn rendered_row(content: &str, row: usize, start_col: usize, options: RenderOptions) -> String {
    let mut out = Vec::new();
    write_content_line(&mut out, content, row, start_col, usize::MAX, options).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn markdown_heading_uses_the_semantic_default() {
    assert_eq!(
        rendered(
            "## Heading",
            0,
            RenderOptions {
                syntax: SyntaxKind::Markdown,
                ..RenderOptions::default()
            }
        ),
        "\x1b[94;1m## Heading\x1b[0m"
    );
}

#[test]
fn markdown_inline_code_is_distinct_from_cyan_markers() {
    assert_eq!(
        rendered(
            "- `code`",
            0,
            RenderOptions {
                syntax: SyntaxKind::Markdown,
                ..RenderOptions::default()
            }
        ),
        "\x1b[96m- \x1b[32m`code`\x1b[0m"
    );
}

#[test]
fn markdown_presentation_uses_attributes_and_osc8_without_source_delimiters() {
    let spans = vec![vec![
        StyledSpan {
            start: 0,
            end: 6,
            style: SpanStyle::PreviewStrong,
        },
        StyledSpan {
            start: 7,
            end: 15,
            style: SpanStyle::PreviewEmphasis,
        },
        StyledSpan {
            start: 16,
            end: 22,
            style: SpanStyle::PreviewStrikethrough,
        },
        StyledSpan {
            start: 23,
            end: 27,
            style: SpanStyle::PreviewLink,
        },
        StyledSpan {
            start: 28,
            end: 32,
            style: SpanStyle::PreviewInlineCode,
        },
    ]];
    let links = vec![vec![HyperlinkSpan {
        start: 23,
        end: 27,
        destination: "https://example.com".into(),
    }]];
    let annotations =
        crate::editor::markdown_preview::MarkdownAnnotations::from_rows(&spans, &links);
    let presentation = super::super::DocumentPresentation {
        annotations: &annotations,
    };

    let output = rendered(
        "strong emphasis strike link code",
        0,
        RenderOptions {
            presentation: Some(presentation),
            surface: ContentSurface::Preview,
            ..RenderOptions::default()
        },
    );

    assert_eq!(
        output,
        concat!(
            "\x1b[35;1mstrong\x1b[39;22m ",
            "\x1b[35;4memphasis\x1b[39;24m ",
            "\x1b[35;9mstrike\x1b[39;29m ",
            "\x1b[0m\x1b]8;;https://example.com\x1b\\",
            "\x1b[94;4mlink\x1b[0m\x1b]8;;\x1b\\ ",
            "\x1b[32;1mcode\x1b[0m"
        )
    );
    assert!(!output.contains("**"));
    assert!(!output.contains("~~"));

    let monochrome = rendered(
        "strong emphasis strike link code",
        0,
        RenderOptions {
            presentation: Some(presentation),
            theme: crate::config::theme::parse("[theme]\nname = 'mono'\n").unwrap(),
            ..RenderOptions::default()
        },
    );
    assert!(monochrome.contains("\x1b[1mstrong\x1b[22m"));
    assert!(monochrome.contains("\x1b[4memphasis\x1b[24m"));
    assert!(monochrome.contains("\x1b[9mstrike\x1b[29m"));
    assert!(monochrome.contains("\x1b[4mlink\x1b[0m"));
    assert!(monochrome.contains("\x1b[1mcode\x1b[0m"));
}

#[test]
fn markdown_code_blocks_use_code_color_without_a_reversed_surface() {
    let spans = vec![vec![StyledSpan {
        start: 0,
        end: 8,
        style: SpanStyle::PreviewCodeBlock,
    }]];
    let links = vec![Vec::new()];
    let annotations =
        crate::editor::markdown_preview::MarkdownAnnotations::from_rows(&spans, &links);
    let presentation = super::super::DocumentPresentation {
        annotations: &annotations,
    };

    let output = rendered(
        "  code  ",
        0,
        RenderOptions {
            presentation: Some(presentation),
            surface: ContentSurface::Preview,
            ..RenderOptions::default()
        },
    );

    assert_eq!(output, "\x1b[32m  code  \x1b[0m");
}

#[test]
fn preview_heading_levels_keep_distinct_monochrome_attributes() {
    let theme = crate::config::theme::parse("[theme]\nname = 'mono'\n").unwrap();
    let h1 = span_style(theme, SpanStyle::PreviewHeading1);
    let h2 = span_style(theme, SpanStyle::PreviewHeading2);
    let h3 = span_style(theme, SpanStyle::PreviewHeading3);
    let h4 = span_style(theme, SpanStyle::PreviewHeading4);
    let h5 = span_style(theme, SpanStyle::PreviewHeading5);
    let h6 = span_style(theme, SpanStyle::PreviewHeading6);

    assert_eq!((h1.bold, h1.reversed), (Some(true), Some(true)));
    assert_eq!((h2.bold, h2.reversed), (Some(true), None));
    assert_eq!((h3.bold, h3.reversed), (Some(false), None));
    assert_eq!((h4.bold, h4.dim), (Some(false), None));
    assert_eq!((h5.bold, h5.dim), (Some(false), Some(true)));
    assert_eq!((h6.bold, h6.dim), (Some(false), Some(true)));
}

#[test]
fn selection_combines_with_keyword_color() {
    let output = rendered(
        "let cat = 1",
        0,
        RenderOptions {
            syntax: SyntaxKind::Rust,
            highlight: Some(TextHighlight {
                start: Cursor { row: 0, col: 0 },
                end: Cursor { row: 0, col: 3 },
            }),
            ..RenderOptions::default()
        },
    );
    assert_eq!(output, "\x1b[30;46mlet\x1b[39;49m cat = \x1b[33m1\x1b[0m");
}

#[test]
fn highlight_maps_through_horizontal_scroll() {
    let output = rendered(
        "cdef",
        2,
        RenderOptions {
            highlight: Some(TextHighlight {
                start: Cursor { row: 0, col: 3 },
                end: Cursor { row: 0, col: 5 },
            }),
            ..RenderOptions::default()
        },
    );
    assert_eq!(output, "c\x1b[30;46mde\x1b[39;49mf");
}

#[test]
fn search_and_selection_use_distinct_semantic_roles() {
    let range = Some(TextHighlight {
        start: Cursor { row: 0, col: 0 },
        end: Cursor { row: 0, col: 3 },
    });
    let search = rendered(
        "cat",
        0,
        RenderOptions {
            highlight: range,
            highlight_kind: HighlightKind::Search,
            ..RenderOptions::default()
        },
    );
    let selection = rendered(
        "cat",
        0,
        RenderOptions {
            highlight: range,
            ..RenderOptions::default()
        },
    );
    assert_eq!(search, "\x1b[30;43mcat\x1b[0m");
    assert_eq!(selection, "\x1b[30;46mcat\x1b[0m");
}

#[test]
fn rgb_uses_truecolor_or_a_stable_indexed_fallback() {
    let theme = Theme {
        text: Style::fg(Color::Rgb(255, 0, 0)),
        ..Theme::default()
    };
    let fallback = rendered(
        "cat",
        0,
        RenderOptions {
            theme,
            ..RenderOptions::default()
        },
    );
    let theme = Theme {
        truecolor: true,
        ..theme
    };
    let truecolor = rendered(
        "cat",
        0,
        RenderOptions {
            theme,
            ..RenderOptions::default()
        },
    );
    assert_eq!(fallback, "\x1b[38;5;196mcat\x1b[0m");
    assert_eq!(truecolor, "\x1b[38;2;255;0;0mcat\x1b[0m");
}

#[test]
fn diff_and_preview_styles_overlay_normal_text() {
    let theme = Theme {
        text: Style::fg(Color::Ansi(7)),
        preview: Style {
            dim: Some(true),
            ..Style::default()
        },
        diff_added: Style::fg(Color::Ansi(10)),
        ..Theme::default()
    };
    assert_eq!(
        rendered(
            "+cat",
            0,
            RenderOptions {
                syntax: SyntaxKind::Diff,
                surface: ContentSurface::Preview,
                theme,
                ..RenderOptions::default()
            }
        ),
        "\x1b[92;2m+cat\x1b[0m"
    );
}

#[test]
fn markdown_table_styling_composes_with_unicode_selection() {
    let output = rendered(
        "| 猫 é | **bold** |",
        0,
        RenderOptions {
            syntax: SyntaxKind::Markdown,
            highlight: Some(TextHighlight {
                start: Cursor { row: 0, col: 2 },
                end: Cursor { row: 0, col: 3 },
            }),
            ..RenderOptions::default()
        },
    );

    assert!(output.contains("\x1b[96m|\x1b[39m \x1b[30;46m猫\x1b[39;49m é "));
    assert!(output.contains("\x1b[35m**bold**\x1b[39m"));
}

#[test]
fn explicit_terminal_default_resets_inherited_overlay_colors() {
    let theme = Theme {
        text: Style::fg(Color::Ansi(1)),
        selection: Style {
            fg: Some(Color::Default),
            reversed: Some(true),
            ..Style::default()
        },
        ..Theme::default()
    };
    assert_eq!(
        rendered(
            "cat",
            0,
            RenderOptions {
                highlight: Some(TextHighlight {
                    start: Cursor { row: 0, col: 0 },
                    end: Cursor { row: 0, col: 3 },
                }),
                theme,
                ..RenderOptions::default()
            }
        ),
        "\x1b[7mcat\x1b[0m"
    );
}

#[test]
fn default_cursor_color_uses_the_terminal_reset_sequence() {
    let mut out = Vec::new();
    write_cursor_color(
        &mut out,
        Theme {
            cursor: Some(Color::Default),
            ..Theme::default()
        },
    )
    .unwrap();
    assert_eq!(out, b"\x1b]112\x07");
}

#[test]
fn line_numbers_inherit_the_base_background() {
    let theme = Theme {
        text: Style::pair(Color::Ansi(7), Color::Ansi(0)),
        line_number: Style::fg(Color::Ansi(6)),
        ..Theme::default()
    };
    let mut out = Vec::new();

    super::super::write_line_number(&mut out, 0, 2, theme).unwrap();

    assert_eq!(String::from_utf8(out).unwrap(), "\x1b[36;40m1 \x1b[0m");
}

#[test]
fn lint_ranges_use_the_distinct_underlined_role() {
    let ranges = [TextHighlight {
        start: Cursor { row: 0, col: 1 },
        end: Cursor { row: 0, col: 2 },
    }];
    let output = rendered(
        "cat",
        0,
        RenderOptions {
            lint_ranges: Some(&ranges),
            ..RenderOptions::default()
        },
    );

    assert_eq!(output, "c\x1b[31;4ma\x1b[39;24mt");
}

#[test]
fn row_indexing_skips_offscreen_sorted_annotations() {
    let ranges = (0..20_000)
        .map(|row| TextHighlight {
            start: Cursor { row, col: 0 },
            end: Cursor { row, col: 1 },
        })
        .collect::<Vec<_>>();

    let visible = super::ranges_for_row(&ranges, 19_999);

    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].start.row, 19_999);
}

#[test]
fn sparse_presentation_and_indexed_ranges_meet_on_a_distant_row() {
    let row = 19_999;
    let mut span_rows = Vec::with_capacity(row + 1);
    span_rows.resize_with(row + 1, Vec::new);
    span_rows[row].push(StyledSpan {
        start: 0,
        end: 1,
        style: SpanStyle::PreviewStrong,
    });
    let annotations =
        crate::editor::markdown_preview::MarkdownAnnotations::from_rows(&span_rows, &[]);
    let presentation = super::super::DocumentPresentation {
        annotations: &annotations,
    };
    let ranges = (0..=row)
        .map(|range_row| TextHighlight {
            start: Cursor {
                row: range_row,
                col: 0,
            },
            end: Cursor {
                row: range_row,
                col: 1,
            },
        })
        .collect::<Vec<_>>();

    let output = rendered_row(
        "x",
        row,
        0,
        RenderOptions {
            presentation: Some(presentation),
            lint_ranges: Some(&ranges),
            ..RenderOptions::default()
        },
    );

    assert_eq!(annotations.annotated_row_count(), 1);
    assert_eq!(annotations.spans(row - 1).count(), 0);
    assert_eq!(super::ranges_for_row(&ranges, row).len(), 1);
    assert!(output.contains("\x1b["), "{output:?}");
    assert!(output.contains('x'), "{output:?}");
}

#[test]
fn overlays_inside_a_combining_grapheme_style_and_link_the_whole_cluster() {
    let spans = vec![vec![StyledSpan {
        start: 1,
        end: 2,
        style: SpanStyle::PreviewLink,
    }]];
    let links = vec![vec![HyperlinkSpan {
        start: 1,
        end: 2,
        destination: "https://example.com".into(),
    }]];
    let annotations =
        crate::editor::markdown_preview::MarkdownAnnotations::from_rows(&spans, &links);
    let presentation = super::super::DocumentPresentation {
        annotations: &annotations,
    };
    let range = TextHighlight {
        start: Cursor { row: 0, col: 1 },
        end: Cursor { row: 0, col: 2 },
    };

    let output = rendered(
        "e\u{301}x",
        0,
        RenderOptions {
            presentation: Some(presentation),
            highlight: Some(range),
            lint_ranges: Some(std::slice::from_ref(&range)),
            external_changes: Some(super::super::ExternalChanges {
                added_ranges: &[],
                changed_ranges: std::slice::from_ref(&range),
                markers: &[],
            }),
            ..RenderOptions::default()
        },
    );

    assert_eq!(
        output,
        "\x1b[0m\x1b]8;;https://example.com\x1b\\\x1b[30;46;4me\u{301}\x1b[0m\
         \x1b]8;;\x1b\\x"
    );
}

#[test]
fn direct_transitions_cover_colors_attributes_and_shared_intensity_reset() {
    let mut out = Vec::new();
    let mut state = StyleState::default();
    state
        .transition(
            &mut out,
            Style {
                fg: Some(Color::Ansi(0)),
                bg: Some(Color::Ansi(15)),
                bold: Some(true),
                dim: Some(true),
                underlined: Some(true),
                reversed: Some(true),
                crossed_out: Some(true),
            },
            true,
        )
        .unwrap();
    state
        .transition(
            &mut out,
            Style {
                fg: Some(Color::Ansi(15)),
                bg: Some(Color::Ansi(0)),
                dim: Some(true),
                ..Style::default()
            },
            true,
        )
        .unwrap();
    state
        .transition(
            &mut out,
            Style::pair(Color::Indexed(201), Color::Indexed(17)),
            true,
        )
        .unwrap();
    state
        .transition(
            &mut out,
            Style::pair(Color::Rgb(1, 2, 3), Color::Rgb(4, 5, 6)),
            true,
        )
        .unwrap();
    state
        .transition(&mut out, Style::pair(Color::Default, Color::Default), true)
        .unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        concat!(
            "\x1b[30;107;1;2;4;7;9m",
            "\x1b[97;40;22;2;24;27;29m",
            "\x1b[38;5;201;48;5;17;22m",
            "\x1b[38;2;1;2;3;48;2;4;5;6m",
            "\x1b[39;49m"
        )
    );
}

#[test]
fn all_ansi_foregrounds_map_to_the_standard_and_bright_ranges() {
    let expected = [
        30u16, 31, 32, 33, 34, 35, 36, 37, 90, 91, 92, 93, 94, 95, 96, 97,
    ];
    for (index, code) in expected.into_iter().enumerate() {
        let mut out = Vec::new();
        write_styled_text(&mut out, "x", Style::fg(Color::Ansi(index as u8)), false).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!("\x1b[{code}mx\x1b[0m")
        );
    }
}

#[test]
fn row_start_clears_with_the_background_then_restores_default_style() {
    let mut plain = Vec::new();
    write_row_start(&mut plain, 3, Style::default(), false).unwrap();
    assert_eq!(plain, b"\x1b[3;1H\x1b[K");

    let mut colored = Vec::new();
    write_row_start(
        &mut colored,
        4,
        Style::pair(Color::Ansi(7), Color::Ansi(0)),
        false,
    )
    .unwrap();
    assert_eq!(colored, b"\x1b[4;1H\x1b[37;40m\x1b[K\x1b[0m");
}

#[test]
fn adjacent_equal_sparse_styles_emit_one_prefix_and_one_boundary_reset() {
    let spans = vec![vec![
        StyledSpan {
            start: 0,
            end: 2,
            style: SpanStyle::PreviewStrong,
        },
        StyledSpan {
            start: 2,
            end: 4,
            style: SpanStyle::PreviewStrong,
        },
    ]];
    let annotations = crate::editor::markdown_preview::MarkdownAnnotations::from_rows(&spans, &[]);
    let output = rendered(
        "same",
        0,
        RenderOptions {
            presentation: Some(super::super::DocumentPresentation {
                annotations: &annotations,
            }),
            surface: ContentSurface::Preview,
            ..RenderOptions::default()
        },
    );

    assert_eq!(output, "\x1b[35;1msame\x1b[0m");
}

#[test]
fn monochrome_plain_text_emits_no_segment_escape_sequences() {
    let theme = crate::config::theme::parse("[theme]\nname = 'mono'\n").unwrap();
    assert_eq!(
        rendered(
            "plain",
            0,
            RenderOptions {
                theme,
                ..RenderOptions::default()
            }
        ),
        "plain"
    );
}

#[test]
fn sgr_emission_allocates_zero_times_for_many_normal_transitions() {
    const SEGMENTS: usize = 512;
    let mut out = Vec::with_capacity(SEGMENTS * 64);
    let first = Style {
        fg: Some(Color::Rgb(1, 2, 3)),
        bg: Some(Color::Indexed(17)),
        bold: Some(true),
        underlined: Some(true),
        ..Style::default()
    };
    let second = Style {
        fg: Some(Color::Ansi(12)),
        bg: Some(Color::Default),
        dim: Some(true),
        reversed: Some(true),
        crossed_out: Some(true),
        ..Style::default()
    };

    let (result, allocations) = count_thread_allocations(|| {
        let mut state = StyleState::default();
        for index in 0..SEGMENTS {
            state.transition(&mut out, if index % 2 == 0 { first } else { second }, true)?;
            out.write_all(b"x")?;
        }
        state.reset(&mut out)
    });
    result.unwrap();

    assert_eq!(allocations, 0);
    assert!(!out.is_empty());
}

#[test]
fn style_heavy_line_has_no_per_segment_allocations() {
    fn sample(segment_count: usize) -> usize {
        let content = "x".repeat(segment_count);
        let spans = vec![(0..segment_count)
            .map(|index| StyledSpan {
                start: index,
                end: index + 1,
                style: SpanStyle::PreviewStrong,
            })
            .collect::<Vec<_>>()];
        let annotations =
            crate::editor::markdown_preview::MarkdownAnnotations::from_rows(&spans, &[]);
        let options = RenderOptions {
            presentation: Some(super::super::DocumentPresentation {
                annotations: &annotations,
            }),
            surface: ContentSurface::Preview,
            ..RenderOptions::default()
        };
        let mut out = Vec::with_capacity(content.len() + 32);
        let (result, allocations) = count_thread_allocations(|| {
            write_content_line(&mut out, &content, 0, 0, usize::MAX, options)
        });
        result.unwrap();
        assert_eq!(
            out.windows(b"\x1b[35;1m".len())
                .filter(|window| *window == b"\x1b[35;1m")
                .count(),
            1
        );
        assert!(out.ends_with(b"\x1b[0m"));
        allocations
    }

    let four_segments = sample(4);
    let many_segments = sample(256);
    assert!(
        many_segments <= four_segments + 32,
        "layout and boundary growth may allocate logarithmically, but style \
         emission must not allocate per segment: \
         four={four_segments}, many={many_segments}"
    );
}
