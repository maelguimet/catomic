//! Purpose: verify the built-in shortcut reference and its read-only lifecycle.
//! Owns: focused help key, navigation, and source-preservation regression tests.
//! Must not: touch disk, spawn services, access network, or depend on a real terminal.
//! Invariants: opening and closing help never changes the source buffer.
//! Phase: post-v0.1 core usability.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::io::{self, Write};

use crate::buffer::{Cursor, PieceTable};

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
fn ctrl_h_commits_help_content_and_status_as_one_frame() {
    let mut app = app();
    app.screen.width = 120;
    app.screen.height = 50;
    let mut out = FrameRecorder::default();

    let toggle = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL);
    assert!(handle_key(&mut app, &mut out, toggle).unwrap());

    assert_eq!(out.writes.len(), 1, "help redraw must be one output frame");
    assert_eq!(out.flushes, 1, "the committed frame must be flushed once");
    let frame = String::from_utf8_lossy(&out.writes[0]);
    assert!(frame.contains("Catomic shortcuts"));
    assert!(frame.contains("Ctrl+Z                  Undo"));
    assert!(frame.contains("Ctrl+Y / Ctrl+Shift+Z   Redo"));
    assert!(!frame.contains("Ctrl+Z/Y"));
    assert!(frame.contains("\x1b[50;1H\x1b[KShortcuts (read-only). Esc or Ctrl+H closes."));
    assert!(
        frame.ends_with("\x1b[1;1H"),
        "frame must include cursor placement"
    );
}

#[test]
fn ctrl_h_opens_navigates_and_closes_without_editing_source() {
    let mut app = app();
    let mut out = Vec::new();
    let toggle = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL);

    assert!(handle_key(&mut app, &mut out, toggle).unwrap());
    assert!(is_viewing(&app));
    assert!(display_buffer(&app)
        .unwrap()
        .to_string()
        .contains("Ctrl+Shift+S"));

    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    )
    .unwrap();
    assert!(display_buffer(&app).unwrap().cursor().row > 0);

    assert!(handle_key(&mut app, &mut out, toggle).unwrap());
    assert!(!is_viewing(&app));
    assert_eq!(app.buffer.to_string(), "source text");
}

#[test]
fn help_rejects_edits_and_escape_restores_source_viewport() {
    let mut app = app();
    let mut out = Vec::new();
    app.buffer = Box::new(PieceTable::from_text("a\nb\nc\nsource"));
    app.buffer.set_cursor(Cursor { row: 3, col: 0 });
    app.screen.height = 2;
    app.screen.scroll_top = 3;
    show(&mut app, &mut out).unwrap();
    let help_before = display_buffer(&app).unwrap().to_string();

    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
    )
    .unwrap();
    assert_eq!(display_buffer(&app).unwrap().to_string(), help_before);

    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    )
    .unwrap();
    assert!(!is_viewing(&app));
    assert_eq!(app.screen.scroll_top, 3);
}
