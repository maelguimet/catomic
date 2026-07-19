//! Purpose: verify quiet identity styles, transient row painting, clipping, and tiny viewports.
//! Owns: deterministic color/monochrome frame assertions for normal and wrapped rendering.
//! Must not: depend on ambient terminal capabilities, launch a PTY, touch disk, or mutate config.
//! Invariants: status tests use explicit themes and assert resets after every styled row.
//! Phase: post-v0.1 status/message bar accessibility.

use crossterm::style::Color;

use super::*;
use crate::buffer::SimpleBuffer;
use crate::editor::syntax::SyntaxKind;
use crate::terminal::render::{render_buffer, RenderOptions, RenderViewport};

fn monochrome_status(role: StatusRole) -> RenderOptions<'static> {
    RenderOptions {
        status_role: role,
        status_theme: StatusTheme::monochrome(),
        ..RenderOptions::default()
    }
}

#[test]
fn terminal_capability_selection_has_a_monochrome_inverse_fallback() {
    assert_eq!(
        StatusTheme::for_terminal(true, Some("xterm-256color")),
        StatusTheme::monochrome()
    );
    assert_eq!(
        StatusTheme::for_terminal(false, Some("dumb")),
        StatusTheme::monochrome()
    );
    assert_eq!(
        StatusTheme::for_terminal(false, Some("xterm-mono")),
        StatusTheme::monochrome()
    );
    assert_eq!(
        StatusTheme::for_terminal(false, Some("vt100")),
        StatusTheme::monochrome()
    );
    assert_eq!(
        StatusTheme::for_terminal(false, None),
        StatusTheme::monochrome()
    );
    assert_eq!(
        StatusTheme::for_terminal(false, Some("linux")),
        StatusTheme::default()
    );
}

#[test]
fn monochrome_messages_keep_a_boundary_while_normal_status_stays_quiet() {
    for role in [
        StatusRole::Normal,
        StatusRole::Info,
        StatusRole::Warning,
        StatusRole::Error,
        StatusRole::Prompt,
    ] {
        let mut out = Vec::new();
        write_status_bar(
            &mut out,
            3,
            4,
            "ok",
            StatusBarPresentation {
                role,
                theme: StatusTheme::monochrome(),
                filename: None,
                selection: None,
            },
        )
        .unwrap();
        let rendered = String::from_utf8(out).unwrap();
        if role == StatusRole::Normal {
            assert!(rendered.contains("\x1b[2m"), "role {role:?}: {rendered:?}");
            assert!(!rendered.contains("\x1b[7m"), "role {role:?}: {rendered:?}");
            assert!(rendered.ends_with("ok\x1b[0m"));
        } else {
            assert!(rendered.contains("\x1b[7m"), "role {role:?}: {rendered:?}");
            assert!(rendered.ends_with("ok  \x1b[0m"));
        }
    }
}

#[test]
fn default_semantic_roles_use_distinct_basic_color_pairs() {
    let cases = [
        (StatusRole::Normal, "\x1b[90m\x1b[2m"),
        (StatusRole::Info, "\x1b[30m\x1b[106m"),
        (StatusRole::Warning, "\x1b[30m\x1b[103m\x1b[1m"),
        (StatusRole::Error, "\x1b[97m\x1b[41m\x1b[1m"),
        (StatusRole::Prompt, "\x1b[97m\x1b[44m\x1b[1m"),
    ];
    for (role, style) in cases {
        let mut out = Vec::new();
        write_status_bar(
            &mut out,
            2,
            2,
            "ok",
            StatusBarPresentation {
                role,
                theme: StatusTheme::default(),
                filename: None,
                selection: None,
            },
        )
        .unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains(style), "role {role:?}: {rendered:?}");
    }
}

#[test]
fn custom_semantic_style_is_used_without_changing_bar_logic() {
    let theme = StatusTheme::monochrome().with_role_colors(
        StatusRole::Error,
        Color::Black,
        Color::Magenta,
        false,
    );
    let mut out = Vec::new();
    write_status_bar(
        &mut out,
        2,
        3,
        "bad",
        StatusBarPresentation {
            role: StatusRole::Error,
            theme,
            filename: None,
            selection: None,
        },
    )
    .unwrap();
    let rendered = String::from_utf8(out).unwrap();

    assert!(rendered.contains("\x1b[30m\x1b[105m"));
    assert!(!rendered.contains("\x1b[7m"));
}

#[test]
fn narrow_unicode_status_is_cell_clipped_and_terminal_safe() {
    let mut out = Vec::new();
    write_status_bar(
        &mut out,
        2,
        4,
        "猫x\x1b[2Jtail",
        StatusBarPresentation {
            role: StatusRole::Warning,
            theme: StatusTheme::monochrome(),
            filename: None,
            selection: None,
        },
    )
    .unwrap();
    let rendered = String::from_utf8(out).unwrap();

    assert!(rendered.contains("\x1b[2K猫…l\x1b[0m"));
    assert!(!rendered.contains("\x1b[2J"));
    assert!(rendered.ends_with("\x1b[0m"));
}

#[test]
fn middle_clipping_keeps_prompt_context_and_actionable_tail() {
    let prompt =
        "Send detailed local context to a configured endpoint? Enter confirms; Esc cancels.";
    let visible = clipped_status_text(prompt, 64);

    assert!(visible.starts_with("Send detailed local context"));
    assert!(visible.contains('…'));
    assert!(visible.ends_with("Enter confirms; Esc cancels."));
    assert_eq!(crate::editor::text_layout::cell_width(&visible), 64);
}

#[test]
fn normal_status_clears_the_row_without_painting_a_full_width_background() {
    let buffer = SimpleBuffer::from_text("text");
    let mut out = Vec::new();
    render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 2, 6),
        Some("ok"),
        monochrome_status(StatusRole::Normal),
    )
    .unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("\x1b[2;1H\x1b[2K\x1b[0m\x1b[2mok\x1b[0m"));
    assert!(!rendered.contains("ok    "));
    assert!(rendered.ends_with("\x1b[0m\x1b[0 q\x1b[1;1H\x1b[?25h"));
}

#[test]
fn persistent_path_uses_muted_parent_red_filename_and_selection_reverse_video() {
    let mut out = Vec::new();
    let text = "=^..^=  /work/note.txt";
    write_status_bar(
        &mut out,
        2,
        40,
        text,
        StatusBarPresentation {
            role: StatusRole::Normal,
            theme: StatusTheme::default(),
            filename: Some((14, 22)),
            selection: Some((8, 22)),
        },
    )
    .unwrap();
    let rendered = String::from_utf8(out).unwrap();

    assert!(rendered.contains("\x1b[90m\x1b[2m=^..^=  "));
    assert!(rendered.contains("\x1b[90m\x1b[7m/work/"));
    assert!(rendered.contains("\x1b[91m\x1b[7mnote.txt"));
    assert!(!rendered.contains("note.txt                  "));
}

#[test]
fn warning_error_and_prompt_roles_have_distinct_monochrome_styles() {
    let cases = [
        (StatusRole::Warning, "\x1b[1m\x1b[7m"),
        (StatusRole::Error, "\x1b[1m\x1b[4m\x1b[7m"),
        (StatusRole::Prompt, "\x1b[4m\x1b[7m"),
    ];
    for (role, style) in cases {
        let buffer = SimpleBuffer::from_text("");
        let mut out = Vec::new();
        render_buffer(
            &mut out,
            &buffer,
            RenderViewport::new(0, 0, 2, 5),
            Some("state"),
            monochrome_status(role),
        )
        .unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains(style), "role {role:?}: {rendered:?}");
        assert!(rendered.contains("\x1b[2Kstate\x1b[0m"));
    }
}

#[test]
fn line_numbered_render_keeps_info_bar_outside_the_gutter() {
    let buffer = SimpleBuffer::from_text("one");
    let mut out = Vec::new();
    render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 2, 8),
        Some("info"),
        RenderOptions {
            line_numbers: true,
            ..monochrome_status(StatusRole::Info)
        },
    )
    .unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("\x1b[1;1H\x1b[K\x1b[90m1 \x1b[0mone"));
    assert!(rendered.contains("\x1b[2;1H\x1b[7m\x1b[2Kinfo    \x1b[0m"));
}

#[test]
fn preview_render_uses_the_semantic_info_bar() {
    let buffer = SimpleBuffer::from_text("▌ Preview");
    let mut out = Vec::new();
    render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 2, 12),
        Some("Preview"),
        RenderOptions {
            syntax: SyntaxKind::MarkdownPreview,
            ..monochrome_status(StatusRole::Info)
        },
    )
    .unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("▌ Preview"));
    assert!(rendered.contains("\x1b[2;1H\x1b[7m\x1b[2KPreview     \x1b[0m"));
}

#[test]
fn status_terminal_controls_render_inertly() {
    let buffer = SimpleBuffer::from_text("");
    let mut out = Vec::new();
    render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 2, 80),
        Some("error from hostile\x1b]0;title\x07path"),
        monochrome_status(StatusRole::Error),
    )
    .unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(!rendered.contains("\x1b]0"));
    assert!(!rendered.contains('\x07'));
    assert!(rendered.contains("error from hostile␛]0;title␇path"));
}

#[test]
fn height_one_reserves_only_row_for_the_status_bar() {
    let buffer = SimpleBuffer::from_text("L0\nL1\nL2\n");
    let mut out = Vec::new();
    render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 1, 10),
        Some("msg"),
        monochrome_status(StatusRole::Info),
    )
    .unwrap();
    let rendered = String::from_utf8(out).unwrap();

    assert!(!rendered.contains("L0") && !rendered.contains("L1") && !rendered.contains("L2"));
    assert!(rendered.contains("\x1b[1;1H\x1b[7m\x1b[2Kmsg"));
}

#[test]
fn width_zero_clears_rows_without_emitting_content_or_style_leaks() {
    let buffer = SimpleBuffer::from_text("abc\ndef\n");
    let mut out = Vec::new();
    render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 3, 0),
        None,
        monochrome_status(StatusRole::Normal),
    )
    .unwrap();
    let rendered = String::from_utf8(out).unwrap();

    assert!(!rendered.contains("abc") && !rendered.contains("def"));
    assert!(rendered.contains("\x1b[1;1H\x1b[K"));
    assert!(rendered.contains("\x1b[2;1H\x1b[K"));
    assert!(rendered.contains("\x1b[3;1H\x1b[2K\x1b[0m"));
    assert!(rendered.ends_with("\x1b[0m\x1b[0 q\x1b[?25l\x1b[1;1H"));
}
