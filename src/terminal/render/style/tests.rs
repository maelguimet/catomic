//! Purpose: verify ANSI composition for syntax and active document ranges.
//! Owns: exact visible-line color, semantic highlights, and scalar-offset fixtures.
//! Must not: query buffers, require a terminal, inspect files, or test syntax detection.
//! Invariants: styled segments end with a full reset so attributes never leak.
//! Phase: 4-a viewport-only syntax styling.

use super::*;
use crate::buffer::Cursor;
use crate::editor::syntax::SyntaxKind;

fn rendered(content: &str, start_col: usize, options: RenderOptions) -> String {
    let mut out = Vec::new();
    write_content_line(&mut out, content, 0, start_col, usize::MAX, options).unwrap();
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
        "\x1b[96m- \x1b[0m\x1b[32m`code`\x1b[0m"
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
            style: SpanStyle::PreviewCode,
        },
    ]];
    let links = vec![vec![HyperlinkSpan {
        start: 23,
        end: 27,
        destination: "https://example.com".into(),
    }]];
    let presentation = super::super::DocumentPresentation {
        spans: &spans,
        links: &links,
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

    assert!(output.contains("\x1b[35;1mstrong\x1b[0m"), "{output:?}");
    assert!(output.contains("\x1b[35;4memphasis\x1b[0m"), "{output:?}");
    assert!(output.contains("\x1b[35;9mstrike\x1b[0m"), "{output:?}");
    assert!(output.contains("\x1b[94;4mlink\x1b[0m"), "{output:?}");
    assert!(output.contains("\x1b[32;7mcode\x1b[0m"), "{output:?}");
    assert!(output.contains("\x1b]8;;https://example.com\x1b\\"));
    assert!(output.contains("\x1b]8;;\x1b\\"));
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
    assert!(monochrome.contains("\x1b[1mstrong\x1b[0m"));
    assert!(monochrome.contains("\x1b[4memphasis\x1b[0m"));
    assert!(monochrome.contains("\x1b[9mstrike\x1b[0m"));
    assert!(monochrome.contains("\x1b[4mlink\x1b[0m"));
    assert!(monochrome.contains("\x1b[7mcode\x1b[0m"));
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
    assert_eq!(output, "\x1b[30;46mlet\x1b[0m cat = \x1b[33m1\x1b[0m");
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
    assert_eq!(output, "c\x1b[30;46mde\x1b[0mf");
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
                surface: ContentSurface::Diff,
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

    assert!(output.contains("\x1b[96m|\x1b[0m \x1b[30;46m猫\x1b[0m é "));
    assert!(output.contains("\x1b[35m**bold**\x1b[0m"));
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
        "\x1b[39;7mcat\x1b[0m"
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
fn llm_changed_ranges_are_red_underlined_and_grapheme_safe() {
    let ranges = [TextHighlight {
        start: Cursor { row: 0, col: 1 },
        end: Cursor { row: 0, col: 3 },
    }];
    let output = rendered(
        "a猫🙂z",
        0,
        RenderOptions {
            llm_changes: Some(super::super::LlmChanges {
                ranges: &ranges,
                gutter_lines: &[0],
            }),
            ..RenderOptions::default()
        },
    );
    assert_eq!(output, "a\x1b[31;4m猫🙂\x1b[0mz");
}

#[test]
fn llm_changed_ranges_have_a_non_color_fallback() {
    let ranges = [TextHighlight {
        start: Cursor { row: 0, col: 1 },
        end: Cursor { row: 0, col: 4 },
    }];
    let output = rendered(
        "cdef",
        2,
        RenderOptions {
            llm_changes: Some(super::super::LlmChanges {
                ranges: &ranges,
                gutter_lines: &[0],
            }),
            theme: crate::config::theme::parse("[theme]\nname = 'mono'\n").unwrap(),
            ..RenderOptions::default()
        },
    );
    assert_eq!(output, "\x1b[4;7mcd\x1b[0mef");
}
