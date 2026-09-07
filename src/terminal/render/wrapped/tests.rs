//! Purpose: verify bounded soft-wrap row splitting, cursor mapping, and rendering.
//! Owns: focused ASCII, Unicode, gutter, and viewport-start fixtures.
//! Must not: mutate App state, access disk/network, or require a real terminal.
//! Invariants: visual rows preserve source text and never split grapheme clusters.

use crate::buffer::{Buffer, Cursor, SimpleBuffer};
use crate::config::theme::{Color, Style, Theme};
use crate::editor::syntax::SyntaxKind;
use std::borrow::Cow;

use super::*;

#[test]
fn visible_rows_wrap_without_changing_document_coordinates() {
    let buffer = SimpleBuffer::from_text("abcdef\n猫猫x");
    let rows = visible_rows(&buffer, 0, 0, 4, 3).unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!((rows[0].document_row, rows[0].start_col), (0, 0));
    assert_eq!(rows[0].content, "abc");
    assert_eq!((rows[1].document_row, rows[1].start_col), (0, 3));
    assert_eq!(rows[1].content, "def");
    assert_eq!((rows[2].document_row, rows[2].start_col), (1, 0));
    assert_eq!(rows[2].content, "猫");
    assert_eq!((rows[3].document_row, rows[3].start_col), (1, 1));
    assert_eq!(rows[3].content, "猫x");
}

#[test]
fn cursor_visibility_stops_at_the_cursor_without_materializing_the_viewport() {
    let buffer = SimpleBuffer::from_text("abc\ndef\nghi\njkl");

    let visibility = cursor_visibility(&buffer, 0, 0, 4, 3).unwrap();

    assert!(visibility.visible);
    assert_eq!(visibility.rows_examined, 1);
}

#[test]
fn wrapped_cursor_uses_the_continuation_row_and_cell_width() {
    let mut buffer = SimpleBuffer::from_text("ab猫x");
    buffer.set_cursor(Cursor { row: 0, col: 3 });
    let rows = visible_rows(&buffer, 0, 0, 3, 3).unwrap();
    assert_eq!(
        wrapped_cursor_position(buffer.cursor(), &rows, 0, 3),
        Some((2, 3))
    );
    assert!(cursor_is_visible(&buffer, 0, 0, 3, 3).unwrap());
}

#[test]
fn wrapped_render_hides_a_document_cursor_above_the_viewport() {
    let mut buffer = SimpleBuffer::from_text("abcdef\nnext");
    buffer.set_cursor(Cursor { row: 0, col: 0 });
    let mut out = Vec::new();

    super::super::render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 3, 3).with_wrap_col(3),
        None,
        RenderOptions {
            soft_wrap: true,
            ..RenderOptions::default()
        },
    )
    .unwrap();

    assert!(String::from_utf8(out)
        .unwrap()
        .ends_with("\x1b[?25l\x1b[1;1H\x1b[?2026l"));
}

#[test]
fn wrapped_render_emits_each_visual_row() {
    let buffer = SimpleBuffer::from_text("abcdef");
    let mut out = Vec::new();
    super::super::render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 4, 3),
        Some("go"),
        RenderOptions {
            soft_wrap: true,
            status_role: super::super::StatusRole::Prompt,
            status_theme: super::super::StatusTheme::monochrome(),
            ..RenderOptions::default()
        },
    )
    .unwrap();
    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("\x1b[1;1H\x1b[Kabc"));
    assert!(rendered.contains("\x1b[2;1H\x1b[Kdef"));
    assert!(rendered.contains("\x1b[4;1H\x1b[4m\x1b[7m\x1b[2Kgo \x1b[0m"));
}

#[test]
fn markdown_styles_do_not_change_soft_wrap_coordinates() {
    let source = "**bold** | 猫";
    let mut buffer = SimpleBuffer::from_text(source);
    buffer.set_cursor(Cursor { row: 0, col: 8 });
    let rows = visible_rows(&buffer, 0, 0, 3, 8).unwrap();

    assert_eq!(rows[0].content, "**bold**");
    assert_eq!(rows[1].start_col, 8);
    assert_eq!(rows[1].content, " | 猫");
    assert_eq!(
        wrapped_cursor_position(buffer.cursor(), &rows, 0, 8),
        Some((2, 1))
    );

    let mut out = Vec::new();
    super::super::render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 4, 8),
        None,
        RenderOptions {
            syntax: SyntaxKind::Markdown,
            soft_wrap: true,
            ..RenderOptions::default()
        },
    )
    .unwrap();
    assert!(String::from_utf8(out)
        .unwrap()
        .contains("\x1b[35m**bold**\x1b[0m"));
}

#[test]
fn wrapped_continuation_gutter_inherits_the_base_background() {
    let buffer = SimpleBuffer::from_text("abcdef");
    let mut out = Vec::new();
    let theme = Theme {
        text: Style::pair(Color::Ansi(7), Color::Ansi(0)),
        line_number: Style::fg(Color::Ansi(6)),
        ..Theme::default()
    };
    super::super::render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 4, 6),
        None,
        RenderOptions {
            soft_wrap: true,
            line_numbers: true,
            theme,
            ..RenderOptions::default()
        },
    )
    .unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("\x1b[2;1H\x1b[37;40m\x1b[K\x1b[0m\x1b[36;40m  \x1b[0m"));
}

#[test]
fn wrapped_render_reuses_each_borrowed_row_layout_for_output_and_cursor() {
    let mut buffer = SimpleBuffer::from_text("a👩\u{200d}💻b");
    buffer.set_cursor(Cursor { row: 0, col: 4 });
    let rows = visible_rows(&buffer, 0, 0, 3, 3).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(matches!(rows[0].content, Cow::Borrowed("a👩\u{200d}💻")));
    assert_eq!(
        wrapped_cursor_position(buffer.cursor(), &rows, 0, 3),
        Some((2, 1))
    );
    drop(rows);

    crate::editor::text_layout::reset_visible_layout_builds();
    let mut out = Vec::new();
    super::super::render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 4, 3),
        None,
        RenderOptions {
            soft_wrap: true,
            ..RenderOptions::default()
        },
    )
    .unwrap();

    assert_eq!(
        crate::editor::text_layout::take_visible_layout_build_counts(),
        (2, 0),
        "each materialized wrapped row must build exactly one shared layout"
    );
    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("a👩\u{200d}💻"));
    assert!(rendered.ends_with("\x1b[0 q\x1b[2;1H\x1b[?25h\x1b[?2026l"));
}

#[test]
fn oversized_zwj_grapheme_advances_once_without_terminal_overflow() {
    let mut buffer = SimpleBuffer::from_text("👩\u{200d}💻");
    buffer.set_cursor(Cursor { row: 0, col: 3 });
    let rows = visible_rows(&buffer, 0, 0, 3, 1).unwrap();

    assert_eq!(rows[0].content, "👩\u{200d}💻");
    assert_eq!(rows[0].end_col(), 3);
    assert_eq!(wrapped_cursor_position(buffer.cursor(), &rows, 0, 1), None);

    let mut out = Vec::new();
    super::super::render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 4, 1),
        None,
        RenderOptions {
            soft_wrap: true,
            ..RenderOptions::default()
        },
    )
    .unwrap();
    let rendered = String::from_utf8(out).unwrap();
    assert!(!rendered.contains("👩\u{200d}💻"));
}

#[test]
fn wrapped_boundary_completion_never_splits_a_long_zwj_cluster() {
    let cluster = format!("👩{}", "\u{200d}👩".repeat(40));
    assert!(cluster.chars().count() > 36);
    let mut buffer = SimpleBuffer::from_text(&format!("{cluster}x"));
    let boundary = cluster.chars().count();
    buffer.set_cursor(Cursor {
        row: 0,
        col: boundary,
    });

    let rows = visible_rows(&buffer, 0, 0, 3, 1).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].content, cluster);
    assert_eq!(rows[0].end_col(), boundary);
    assert_eq!(rows[1].start_col, boundary);
    assert_eq!(rows[1].content, "x");
    assert_eq!(
        wrapped_cursor_position(buffer.cursor(), &rows, 0, 1),
        Some((2, 1))
    );
    drop(rows);

    crate::editor::text_layout::reset_visible_layout_builds();
    let mut out = Vec::new();
    super::super::render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 4, 1),
        None,
        RenderOptions {
            soft_wrap: true,
            ..RenderOptions::default()
        },
    )
    .unwrap();

    assert_eq!(
        crate::editor::text_layout::take_visible_layout_build_counts(),
        (2, 2),
        "two discarded probes establish the long cluster boundary; final row layouts are reused"
    );
    let rendered = String::from_utf8(out).unwrap();
    assert!(
        !rendered.contains('👩'),
        "oversized cluster must not overflow"
    );
    assert!(rendered.contains('x'));
}

#[test]
fn reveal_packs_actual_rows_for_wide_graphemes_and_tabs() {
    for text in [
        "猫猫猫猫",
        "🙂🙂🙂🙂",
        "a\t猫🙂bc\t猫",
        "a\u{301}猫👩\u{200d}💻z",
    ] {
        for width in 1..=8 {
            for height in 1..=4 {
                let mut buffer = SimpleBuffer::from_text(text);
                buffer.set_cursor(Cursor {
                    row: 0,
                    col: text.chars().count(),
                });
                let origin =
                    start_col_near_cursor(&buffer, buffer.cursor(), height, width).unwrap();
                let rows = visible_rows(&buffer, 0, origin, height, width).unwrap();
                assert!(
                    wrapped_cursor_position(buffer.cursor(), &rows, 0, width).is_some(),
                    "text={text:?}, width={width}, height={height}, origin={origin}"
                );
                assert_eq!(buffer.grapheme_range(0, origin).unwrap().start, origin);
            }
        }
    }
}

#[test]
fn reveal_layout_work_stays_near_the_cursor_on_a_long_line() {
    let text = format!("{}a{}猫", "x".repeat(100_000), "\u{301}".repeat(100));
    let mut buffer = crate::buffer::PieceTable::from_text(&text);
    buffer.set_cursor(Cursor {
        row: 0,
        col: text.chars().count(),
    });
    crate::editor::text_layout::reset_visible_layout_builds();
    let origin = start_col_near_cursor(&buffer, buffer.cursor(), 2, 3).unwrap();
    let (builds, _) = crate::editor::text_layout::take_visible_layout_build_counts();
    assert!(builds <= 6, "reveal must not plan the logical-line prefix");
    assert_eq!(buffer.grapheme_range(0, origin).unwrap().start, origin);
    let rows = visible_rows(&buffer, 0, origin, 2, 3).unwrap();
    assert!(wrapped_cursor_position(buffer.cursor(), &rows, 0, 3).is_some());
}

#[test]
fn visibility_agrees_with_output_for_an_oversized_final_grapheme() {
    let mut buffer = SimpleBuffer::from_text("猫");
    buffer.set_cursor(Cursor { row: 0, col: 1 });
    assert!(!cursor_is_visible(&buffer, 0, 0, 2, 1).unwrap());
    let origin = start_col_near_cursor(&buffer, buffer.cursor(), 2, 1).unwrap();
    assert!(cursor_is_visible(&buffer, 0, origin, 2, 1).unwrap());
}
