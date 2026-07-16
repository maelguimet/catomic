//! Purpose: provide standard line, page, document, and word navigation shortcuts.
//! Owns: cursor target calculation and Ctrl+Backspace/Delete word edits.
//! Must not: decode terminal bytes, scan whole documents, save, or start background work.
//! Invariants: targets are scalar-coordinate boundaries; word deletion is one undoable edit.
//! Phase: post-v0.1 core usability.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::Cursor;

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    let extend = key.modifiers.contains(KeyModifiers::SHIFT);
    let command = key.modifiers.contains(KeyModifiers::CONTROL);
    let no_extra = !key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    let target = match key.code {
        KeyCode::Home if command => Some(Cursor::default()),
        KeyCode::End if command => Some(document_end(app)),
        KeyCode::Home if no_extra => Some(line_edge(app, false)),
        KeyCode::End if no_extra => Some(line_edge(app, true)),
        KeyCode::PageUp if no_extra => Some(page_target(app, false)),
        KeyCode::PageDown if no_extra => Some(page_target(app, true)),
        KeyCode::Left if command && !key.modifiers.contains(KeyModifiers::ALT) => {
            Some(word_left(app))
        }
        KeyCode::Right if command && !key.modifiers.contains(KeyModifiers::ALT) => {
            Some(word_right(app))
        }
        KeyCode::Backspace if command && !extend => {
            delete_to(app, out, word_left(app))?;
            return Ok(true);
        }
        KeyCode::Delete if command && !extend => {
            delete_to(app, out, word_right(app))?;
            return Ok(true);
        }
        _ => None,
    };
    let Some(target) = target else {
        return Ok(false);
    };
    super::selection::move_to(app, out, target, extend)?;
    Ok(true)
}

fn line_edge(app: &super::App, end: bool) -> Cursor {
    let current = app.buffer.cursor();
    Cursor {
        row: current.row,
        col: if end {
            app.buffer.line_char_count(current.row).unwrap_or(0)
        } else {
            0
        },
    }
}

fn document_end(app: &super::App) -> Cursor {
    let row = app.buffer.line_count().saturating_sub(1);
    Cursor {
        row,
        col: app.buffer.line_char_count(row).unwrap_or(0),
    }
}

fn page_target(app: &super::App, down: bool) -> Cursor {
    let current = app.buffer.cursor();
    let distance = app.screen.visible_height().max(1);
    let last = app.buffer.line_count().saturating_sub(1);
    let row = if down {
        current.row.saturating_add(distance).min(last)
    } else {
        current.row.saturating_sub(distance)
    };
    Cursor {
        row,
        col: current
            .col
            .min(app.buffer.line_char_count(row).unwrap_or(0)),
    }
}

fn word_left(app: &super::App) -> Cursor {
    let current = app.buffer.cursor();
    if current.col == 0 {
        if current.row == 0 {
            return current;
        }
        let row = current.row - 1;
        return Cursor {
            row,
            col: app.buffer.line_char_count(row).unwrap_or(0),
        };
    }
    let chars: Vec<char> = app
        .buffer
        .line(current.row)
        .unwrap_or_default()
        .chars()
        .collect();
    let mut col = current.col.min(chars.len());
    while col > 0 && chars[col - 1].is_whitespace() {
        col -= 1;
    }
    if col > 0 {
        let class = word_class(chars[col - 1]);
        while col > 0 && word_class(chars[col - 1]) == class {
            col -= 1;
        }
    }
    Cursor {
        row: current.row,
        col,
    }
}

fn word_right(app: &super::App) -> Cursor {
    let current = app.buffer.cursor();
    let chars: Vec<char> = app
        .buffer
        .line(current.row)
        .unwrap_or_default()
        .chars()
        .collect();
    let mut col = current.col.min(chars.len());
    if col == chars.len() {
        let last = app.buffer.line_count().saturating_sub(1);
        return if current.row < last {
            Cursor {
                row: current.row + 1,
                col: 0,
            }
        } else {
            current
        };
    }
    if chars[col].is_whitespace() {
        while col < chars.len() && chars[col].is_whitespace() {
            col += 1;
        }
    } else {
        let class = word_class(chars[col]);
        while col < chars.len() && word_class(chars[col]) == class {
            col += 1;
        }
        while col < chars.len() && chars[col].is_whitespace() {
            col += 1;
        }
    }
    Cursor {
        row: current.row,
        col,
    }
}

fn word_class(ch: char) -> u8 {
    if ch.is_alphanumeric() || ch == '_' {
        0
    } else if ch.is_whitespace() {
        1
    } else {
        2
    }
}

fn delete_to(app: &mut super::App, out: &mut dyn Write, target: Cursor) -> io::Result<()> {
    if super::selection::replace_active(app, "")? {
        return super::input::finish_content_edit(app, out);
    }
    let current = app.buffer.cursor();
    let (start, end) = if (target.row, target.col) < (current.row, current.col) {
        (target, current)
    } else {
        (current, target)
    };
    app.buffer.replace_range(start, end, "")?;
    super::input::finish_content_edit(app, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn app(text: &str) -> super::super::App {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text(text));
        app
    }

    #[test]
    fn home_end_and_page_keys_move_and_clamp() {
        let text = (0..40)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut app = app(&text);
        let mut out = Vec::new();
        app.buffer.set_cursor(Cursor { row: 25, col: 4 });

        handle_key(&mut app, &mut out, key(KeyCode::Home, KeyModifiers::NONE)).unwrap();
        assert_eq!(app.buffer.cursor(), Cursor { row: 25, col: 0 });
        handle_key(&mut app, &mut out, key(KeyCode::End, KeyModifiers::NONE)).unwrap();
        assert_eq!(app.buffer.cursor().col, 7);
        handle_key(&mut app, &mut out, key(KeyCode::PageUp, KeyModifiers::NONE)).unwrap();
        assert_eq!(app.buffer.cursor().row, 2);
        handle_key(
            &mut app,
            &mut out,
            key(KeyCode::PageDown, KeyModifiers::NONE),
        )
        .unwrap();
        assert_eq!(app.buffer.cursor().row, 25);
    }

    #[test]
    fn control_arrows_move_by_word_and_shift_extends_selection() {
        let mut app = app("one  two!! three");
        let mut out = Vec::new();

        handle_key(
            &mut app,
            &mut out,
            key(KeyCode::Right, KeyModifiers::CONTROL),
        )
        .unwrap();
        assert_eq!(app.buffer.cursor().col, 5);
        handle_key(
            &mut app,
            &mut out,
            key(KeyCode::Right, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
        )
        .unwrap();
        assert_eq!(app.selection.active().unwrap().ordered().0.col, 5);
        assert_eq!(app.selection.active().unwrap().ordered().1.col, 8);
    }

    #[test]
    fn control_backspace_and_delete_are_single_undoable_edits() {
        let mut app = app("one two three");
        let mut out = Vec::new();
        app.buffer.set_cursor(Cursor { row: 0, col: 8 });

        handle_key(
            &mut app,
            &mut out,
            key(KeyCode::Backspace, KeyModifiers::CONTROL),
        )
        .unwrap();
        assert_eq!(app.buffer.to_string(), "one three");
        app.buffer.undo();
        assert_eq!(app.buffer.to_string(), "one two three");
        app.buffer.set_cursor(Cursor { row: 0, col: 4 });
        handle_key(
            &mut app,
            &mut out,
            key(KeyCode::Delete, KeyModifiers::CONTROL),
        )
        .unwrap();
        assert_eq!(app.buffer.to_string(), "one three");
    }
}
