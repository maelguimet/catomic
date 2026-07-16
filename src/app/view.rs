//! Purpose: own non-mutating per-buffer display toggles and their key bindings.
//! Owns: F7 line-number and F8 whitespace state, messages, and effective content width.
//! Must not: mutate buffer text/history, parse Markdown, perform I/O beyond render, or network.
//! Invariants: toggles are per buffer; gutter width is excluded from content coordinates.
//! Phase: 4-b line-number and whitespace indicators.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ViewOptions {
    pub(crate) line_numbers: bool,
    pub(crate) whitespace: bool,
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    let (label, enabled) = match key.code {
        KeyCode::F(7) => {
            app.view.line_numbers = !app.view.line_numbers;
            ("Line numbers", app.view.line_numbers)
        }
        KeyCode::F(8) => {
            app.view.whitespace = !app.view.whitespace;
            ("Whitespace indicators", app.view.whitespace)
        }
        _ => return Ok(false),
    };
    app.message = Some(format!("{label} {}.", if enabled { "on" } else { "off" }));
    app.reveal_cursor();
    app.render(out)?;
    Ok(true)
}

pub(crate) fn gutter_width(app: &super::App) -> usize {
    app.view
        .line_numbers
        .then(|| crate::terminal::render::line_number_gutter(app.buffer.line_count()))
        .unwrap_or(0)
}

pub(crate) fn content_width(app: &super::App) -> usize {
    (app.screen.width as usize).saturating_sub(gutter_width(app))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn function_keys_toggle_view_state_and_render_indicators() {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("a b\tc"));
        let mut out = Vec::new();

        handle_key(
            &mut app,
            &mut out,
            KeyEvent::new(KeyCode::F(7), KeyModifiers::NONE),
        )
        .unwrap();
        assert!(app.view.line_numbers);
        assert!(String::from_utf8_lossy(&out).contains("1 "));

        out.clear();
        handle_key(
            &mut app,
            &mut out,
            KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE),
        )
        .unwrap();
        assert!(app.view.whitespace);
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("a·b→c"));
    }
}
