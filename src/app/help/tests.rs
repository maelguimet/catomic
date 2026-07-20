//! Purpose: verify curated help content, configured chords, search, and read-only lifetime.
//! Owns: focused help rendering, navigation, and source-preservation regression tests.
//! Must not: touch disk, spawn services, access network, or depend on a real terminal.
//! Invariants: opening and closing help never changes the source buffer.
//! Phase: issue #134 task-oriented Markdown help.

use crate::buffer::{Cursor, PieceTable};
use crate::editor::syntax::SyntaxKind;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::io::{self, Write};

use super::*;

fn app() -> crate::app::App {
    let mut app = crate::app::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("source text"));
    app
}

#[derive(Default)]
struct FrameRecorder {
    writes: Vec<Vec<u8>>,
    flushes: usize,
}

impl Write for FrameRecorder {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.writes.push(buffer.to_vec());
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

#[test]
fn ctrl_h_renders_curated_markdown_as_one_frame() {
    let mut app = app();
    app.screen.width = 120;
    app.screen.height = 50;
    let mut out = FrameRecorder::default();

    let toggle = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL);
    assert!(handle_key(&mut app, &mut out, toggle).unwrap());

    assert_eq!(out.writes.len(), 1, "help redraw must be one output frame");
    assert_eq!(out.flushes, 1, "the committed frame must be flushed once");
    let frame = String::from_utf8_lossy(&out.writes[0]);
    let help = display_buffer(&app).unwrap().to_string();
    assert!(frame.contains("Catomic"));
    assert!(frame.contains("\x1b[94;1m"));
    assert!(help.contains("Catomic help"));
    assert!(help.contains("Files and buffers"));
    assert!(!help.contains("# Catomic help"));
    assert!(help.contains("Ctrl+S"));
    assert!(help.contains("• Save"));
    assert!(!help.contains("**Save**"));
    assert_eq!(
        crate::app::view::display_syntax(&app),
        SyntaxKind::MarkdownPreview
    );
    assert!(frame.contains("\x1b[50;1H"));
    assert!(frame.contains("\x1b[2KHelp; Esc closes."));
    assert!(
        frame.ends_with("\x1b[0m\x1b[0 q\x1b[1;1H\x1b[?25h\x1b[?2026l"),
        "frame must reset styling, select the default cursor, place it, and show it"
    );
}

#[test]
fn help_is_short_task_oriented_and_excludes_registry_clutter() {
    let markdown = help_markdown(&KeyBindings::default());

    for required in [
        "# Catomic help",
        "## Files and buffers",
        "## Edit and navigate",
        "## Commands and views",
        "## External changes and recovery",
        "## Models",
        "Save As",
        "Previous buffer",
        "Command palette",
        "Markdown preview",
        "Dirty buffers are never replaced automatically",
        "`.catnap`",
        "never auto-saved",
        "[user guide](https://github.com/maelguimet/catomic/blob/master/docs/user-guide.md)",
    ] {
        assert!(markdown.contains(required), "help is missing {required:?}");
    }
    for forbidden in [
        "move-left",
        "move-right",
        "mouse-place-cursor",
        "[editor,preview",
        "basic cursor",
        "Copy",
        "Paste",
        "[keybindings]",
        "api_key_env",
        "[[llm.backends]]",
        "gitmeow INSTRUCTION",
    ] {
        assert!(
            !markdown.contains(forbidden),
            "help contains forbidden clutter {forbidden:?}"
        );
    }
    assert!(markdown.lines().count() < 60, "help should remain compact");
}

#[test]
fn help_uses_configured_chords_and_does_not_advertise_unbound_defaults() {
    let bindings = crate::config::keybindings::parse(
        "[keybindings]\nsave = [\"alt+s\"]\nredo = []\ncommand-prompt = [\"f4\"]\n",
    )
    .unwrap();
    let markdown = help_markdown(&bindings);

    assert!(markdown.contains("**Save** (`Alt+S`)"));
    assert!(!markdown.contains("`Ctrl+S`"));
    assert!(markdown.contains("**Redo** — Redo"));
    assert!(!markdown.contains("Ctrl+Y"));
    assert!(!markdown.contains("Ctrl+Shift+Z"));
    assert!(markdown.contains("**Command palette** (`F4`)"));
    assert!(!markdown.contains("Ctrl+Shift+P"));
    assert!(!markdown.contains("`F2`"));
}

#[test]
fn help_search_uses_the_effective_find_binding_and_highlights_a_match() {
    let mut app = app();
    app.keybindings =
        crate::config::keybindings::parse("[keybindings]\nsearch = [\"alt+f\"]\n").unwrap();
    app.screen.width = 80;
    app.screen.height = 12;
    let mut out = Vec::new();
    show(&mut app, &mut out).unwrap();

    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT),
    )
    .unwrap();
    for ch in "recovery".chars() {
        handle_key(
            &mut app,
            &mut out,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        )
        .unwrap();
    }

    assert!(is_viewing(&app));
    assert!(active_search_match(&app).is_some());
    assert!(app.screen.scroll_top > 0);
    assert!(String::from_utf8(out).unwrap().contains("recovery"));
    assert_eq!(app.buffer.to_string(), "source text");
}

#[test]
fn narrow_help_soft_wraps_and_scrolls_without_horizontal_movement() {
    let mut app = app();
    app.screen.width = 24;
    app.screen.height = 7;
    let mut out = Vec::new();
    show(&mut app, &mut out).unwrap();

    let target_row = display_buffer(&app)
        .unwrap()
        .to_string()
        .lines()
        .position(|line| line.contains("Model requests show"))
        .expect("compact model guidance must be present");
    app.surfaces
        .help
        .as_mut()
        .unwrap()
        .buffer
        .set_cursor(Cursor {
            row: target_row,
            col: 0,
        });
    out.clear();
    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
    )
    .unwrap();

    assert!(crate::app::view::soft_wrap_active(&app));
    assert!(app.screen.scroll_top > 0);
    assert_eq!(app.screen.scroll_left, 0);
    assert!(!out.is_empty());
    assert_eq!(app.buffer.to_string(), "source text");
}

#[test]
fn escape_closes_help_without_leaving_a_message_and_restores_source_viewport() {
    let mut app = app();
    let mut out = Vec::new();
    app.buffer = Box::new(PieceTable::from_text("a\nb\nc\nsource"));
    app.buffer.set_cursor(Cursor { row: 3, col: 3 });
    app.view.soft_wrap = true;
    app.screen.width = 20;
    app.screen.height = 2;
    app.screen.scroll_top = 3;
    app.screen.wrap_col = 2;
    show(&mut app, &mut out).unwrap();
    let help_before = display_buffer(&app).unwrap().to_string();

    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
    )
    .unwrap();
    assert_eq!(display_buffer(&app).unwrap().to_string(), help_before);

    out.clear();
    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    )
    .unwrap();
    assert!(!is_viewing(&app));
    assert_eq!(app.message, None);
    assert_eq!(app.screen.scroll_top, 3);
    assert_eq!(app.screen.wrap_col, 2);
    assert!(!String::from_utf8(out).unwrap().contains("Help closed"));
}
