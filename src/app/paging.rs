//! Purpose: connect paged Buffer navigation to App viewport, messages, and render.
//! Owns: Ctrl+PageUp/PageDown direction effects and page-boundary feedback.
//! Must not: scan descriptors directly, mutate file/dirty state, edit, save, or reload.
//! Invariants: successful page changes reset both scroll axes; this module does not mutate
//!   confirmation state itself; non-paged buffers ignore page commands.

use std::io::{self, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PageDirection {
    Previous,
    Next,
}

pub(super) fn handle_page_key(
    app: &mut super::App,
    out: &mut dyn Write,
    direction: PageDirection,
) -> io::Result<()> {
    if app.buffer.page_info().is_none() {
        return Ok(());
    }

    let changed = match direction {
        PageDirection::Previous => app.buffer.previous_page(),
        PageDirection::Next => app.buffer.next_page(),
    };
    match changed {
        Ok(true) => finish_page_change(app),
        Ok(false) => {
            let edge = match direction {
                PageDirection::Previous => "first",
                PageDirection::Next => "last",
            };
            app.message_info(format!("Already on the {edge} file page."));
        }
        Err(error) => app.message_error(format!("Page error: {error}")),
    }
    app.reveal_cursor();
    app.render(out)
}

fn finish_page_change(app: &mut super::App) {
    app.selection.clear();
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    if !app.pending_quit_confirm
        && app.pending_save_conflict.is_none()
        && app.pending_reload.is_none()
    {
        app.message = None;
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn temp_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("catomic_page_keys_{}.txt", std::process::id()))
    }

    #[test]
    fn ctrl_page_keys_change_pages_reset_scroll_and_render_page_status() {
        let path = temp_path();
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "zero\none\ntwo").unwrap();
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::LargeFileBuffer::open_paged(&path, 1).unwrap());
        app.message_warning("initial warning");
        app.screen.scroll_top = 4;
        app.screen.scroll_left = 5;
        let mut out = Vec::new();

        app.handle_key_with(&mut out, key(KeyCode::PageDown))
            .unwrap();

        assert_eq!(app.buffer.line(0).as_deref(), Some("one"));
        assert_eq!(app.buffer.page_info().unwrap().page_number, 2);
        assert_eq!(app.screen.scroll_top, 0);
        assert_eq!(app.screen.scroll_left, 0);
        assert!(app.message.is_none());
        assert!(String::from_utf8_lossy(&out).contains("page 2"));

        out.clear();
        app.handle_key_with(&mut out, key(KeyCode::PageUp)).unwrap();
        assert_eq!(app.buffer.line(0).as_deref(), Some("zero"));
        assert_eq!(app.buffer.page_info().unwrap().page_number, 1);

        app.handle_key_with(&mut out, key(KeyCode::PageUp)).unwrap();
        assert!(app
            .message
            .as_deref()
            .unwrap_or("")
            .contains("first file page"));

        let _ = std::fs::remove_file(path);
    }
}
