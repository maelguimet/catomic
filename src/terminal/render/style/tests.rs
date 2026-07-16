//! Purpose: verify ANSI composition for syntax and active document ranges.
//! Owns: exact visible-line color, reverse-video, and scalar-offset fixtures.
//! Must not: query buffers, require a terminal, inspect files, or test syntax detection.
//! Invariants: styled segments end with a full reset so attributes never leak.
//! Phase: 4-a viewport-only syntax styling.

use super::*;
use crate::buffer::Cursor;
use crate::editor::syntax::SyntaxKind;

fn rendered(content: &str, start_col: usize, options: RenderOptions) -> String {
    let mut out = Vec::new();
    write_content_line(&mut out, content, 0, start_col, options).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn markdown_heading_is_bold_cyan() {
    assert_eq!(
        rendered(
            "## Heading",
            0,
            RenderOptions {
                syntax: SyntaxKind::Markdown,
                ..RenderOptions::default()
            }
        ),
        "\x1b[1;36m## Heading\x1b[0m"
    );
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
        },
    );
    assert_eq!(output, "\x1b[35;7mlet\x1b[0m cat = \x1b[33m1\x1b[0m");
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
    assert_eq!(output, "c\x1b[7mde\x1b[27mf");
}
