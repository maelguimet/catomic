//! Purpose: provide standard line, page, document, and word navigation shortcuts.
//! Owns: cursor target calculation and Ctrl+Backspace/Delete word edits.
//! Must not: decode terminal bytes, scan whole documents, save, or start background work.
//! Invariants: targets are scalar-coordinate boundaries; word deletion is one undoable edit.

use std::io::{self, Write};

#[cfg(test)]
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Buffer, Cursor};
use crate::config::actions::Action;
use crate::editor::text_layout;

mod paragraph;

const GRAPHEME_WINDOW: usize = 64;
const WORD_WINDOW: usize = 256;

#[cfg(test)]
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
        KeyCode::Up if command && !extend && !key.modifiers.contains(KeyModifiers::ALT) => {
            Some(paragraph::target(app, paragraph::Direction::Previous)?)
        }
        KeyCode::Down if command && !extend && !key.modifiers.contains(KeyModifiers::ALT) => {
            Some(paragraph::target(app, paragraph::Direction::Next)?)
        }
        KeyCode::Home if command => Some(Cursor::default()),
        KeyCode::End if command => Some(document_end(app)),
        KeyCode::Home if no_extra => Some(line_edge(app, false)),
        KeyCode::End if no_extra => Some(line_edge(app, true)),
        KeyCode::PageUp if no_extra => Some(page_target(app, false)),
        KeyCode::PageDown if no_extra => Some(page_target(app, true)),
        KeyCode::Left if command && !key.modifiers.contains(KeyModifiers::ALT) => {
            Some(word_left(app)?)
        }
        KeyCode::Right if command && !key.modifiers.contains(KeyModifiers::ALT) => {
            Some(word_right(app)?)
        }
        KeyCode::Backspace if command && !extend => {
            delete_to(app, out, word_left(app)?)?;
            return Ok(true);
        }
        KeyCode::Delete if command && !extend => {
            delete_to(app, out, word_right(app)?)?;
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

pub(crate) fn dispatch_action(
    app: &mut super::App,
    out: &mut dyn Write,
    action: Action,
) -> io::Result<bool> {
    let (target, extend) = match action {
        Action::ParagraphPrevious => (
            paragraph::target(app, paragraph::Direction::Previous)?,
            false,
        ),
        Action::ParagraphNext => (paragraph::target(app, paragraph::Direction::Next)?, false),
        Action::LineStart => (line_edge(app, false), false),
        Action::LineEnd => (line_edge(app, true), false),
        Action::SelectLineStart => (line_edge(app, false), true),
        Action::SelectLineEnd => (line_edge(app, true), true),
        Action::DocumentStart => (Cursor::default(), false),
        Action::DocumentEnd => (document_end(app), false),
        Action::SelectDocumentStart => (Cursor::default(), true),
        Action::SelectDocumentEnd => (document_end(app), true),
        Action::ViewportUp => (page_target(app, false), false),
        Action::ViewportDown => (page_target(app, true), false),
        Action::SelectViewportUp => (page_target(app, false), true),
        Action::SelectViewportDown => (page_target(app, true), true),
        Action::WordLeft => (word_left(app)?, false),
        Action::WordRight => (word_right(app)?, false),
        Action::SelectWordLeft => (word_left(app)?, true),
        Action::SelectWordRight => (word_right(app)?, true),
        Action::DeleteWordBackward => {
            delete_to(app, out, word_left(app)?)?;
            return Ok(true);
        }
        Action::DeleteWordForward => {
            delete_to(app, out, word_right(app)?)?;
            return Ok(true);
        }
        _ => return Ok(false),
    };
    super::selection::move_to(app, out, target, extend)?;
    Ok(true)
}

pub(crate) fn move_grapheme(app: &mut super::App, right: bool) -> io::Result<()> {
    let target = if right {
        next_grapheme_cursor(&*app.buffer)?
    } else {
        previous_grapheme_cursor(&*app.buffer)?
    };
    app.buffer.set_cursor(target);
    Ok(())
}

pub(crate) fn delete_grapheme(
    app: &mut super::App,
    out: &mut dyn Write,
    forward: bool,
) -> io::Result<()> {
    if super::selection::replace_active(app, "")? {
        return super::input::finish_content_edit(app, out);
    }
    let target = if forward {
        next_grapheme_cursor(&*app.buffer)?
    } else {
        previous_grapheme_cursor(&*app.buffer)?
    };
    let current = app.buffer.cursor();
    let scalar_count = if target.row == current.row {
        target.col.abs_diff(current.col).max(1)
    } else {
        1
    };
    for _ in 0..scalar_count {
        if forward {
            app.buffer.delete_forward();
        } else {
            app.buffer.delete_back();
        }
    }
    super::input::finish_content_edit(app, out)
}

pub(crate) fn snap_current_grapheme(app: &mut super::App) -> io::Result<()> {
    let cursor = app.buffer.cursor();
    let col = snap_buffer_col(&*app.buffer, cursor.row, cursor.col)?;
    app.buffer.set_cursor(Cursor {
        row: cursor.row,
        col,
    });
    Ok(())
}

pub(super) fn previous_grapheme_cursor(buffer: &dyn Buffer) -> io::Result<Cursor> {
    let cursor = buffer.cursor();
    if cursor.col == 0 {
        if cursor.row == 0 {
            return Ok(cursor);
        }
        let row = cursor.row - 1;
        return Ok(Cursor {
            row,
            col: buffer.line_char_count(row).unwrap_or(0),
        });
    }
    let mut width = GRAPHEME_WINDOW.min(cursor.col);
    loop {
        let start = cursor.col - width;
        let text = line_window(buffer, cursor.row, start, width)?;
        let local = text_layout::previous_grapheme_col(&text, width);
        if local > 0 || start == 0 {
            return Ok(Cursor {
                row: cursor.row,
                col: start.saturating_add(local),
            });
        }
        width = width.saturating_mul(2).min(cursor.col);
    }
}

pub(super) fn next_grapheme_cursor(buffer: &dyn Buffer) -> io::Result<Cursor> {
    let cursor = buffer.cursor();
    let line_len = buffer.line_char_count(cursor.row).unwrap_or(0);
    if cursor.col >= line_len {
        let last = buffer.line_count().saturating_sub(1);
        return Ok(if cursor.row < last {
            Cursor {
                row: cursor.row + 1,
                col: 0,
            }
        } else {
            cursor
        });
    }
    let remaining = line_len - cursor.col;
    let mut width = GRAPHEME_WINDOW.min(remaining);
    loop {
        let text = line_window(buffer, cursor.row, cursor.col, width)?;
        let local = text_layout::next_grapheme_col(&text, 0);
        if local < text.chars().count() || width == remaining {
            return Ok(Cursor {
                row: cursor.row,
                col: cursor.col.saturating_add(local),
            });
        }
        width = width.saturating_mul(2).min(remaining);
    }
}

fn snap_buffer_col(buffer: &dyn Buffer, row: usize, col: usize) -> io::Result<usize> {
    let line_len = buffer.line_char_count(row).unwrap_or(0);
    let col = col.min(line_len);
    if col == 0 || col == line_len {
        return Ok(col);
    }
    let mut before = GRAPHEME_WINDOW.min(col);
    loop {
        let start = col - before;
        let width = before.saturating_add(GRAPHEME_WINDOW).min(line_len - start);
        let text = line_window(buffer, row, start, width)?;
        let local = text_layout::snap_to_grapheme_col(&text, before);
        if local > 0 || start == 0 {
            return Ok(start.saturating_add(local));
        }
        before = before.saturating_mul(2).min(col);
    }
}

fn line_window(
    buffer: &dyn Buffer,
    row: usize,
    start_col: usize,
    width: usize,
) -> io::Result<String> {
    Ok(buffer
        .try_visible_lines_window(row, 1, start_col, width)?
        .into_iter()
        .next()
        .map(|line| line.content)
        .unwrap_or_default())
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

fn word_left(app: &super::App) -> io::Result<Cursor> {
    let current = app.buffer.cursor();
    if current.col == 0 {
        if current.row == 0 {
            return Ok(current);
        }
        let row = current.row - 1;
        return Ok(Cursor {
            row,
            col: app.buffer.line_char_count(row).unwrap_or(0),
        });
    }

    let line_len = app.buffer.line_char_count(current.row).unwrap_or(0);
    let mut col = current.col.min(line_len);
    let mut class = None;
    'chunks: while col > 0 {
        let start = col.saturating_sub(WORD_WINDOW);
        let text = line_window(&*app.buffer, current.row, start, col - start)?;
        for ch in text.chars().rev() {
            if class.is_none() && ch.is_whitespace() {
                col -= 1;
                continue;
            }
            let ch_class = word_class(ch);
            match class {
                None => class = Some(ch_class),
                Some(expected) if expected != ch_class => break 'chunks,
                Some(_) => {}
            }
            col -= 1;
        }
        if start == 0 {
            break;
        }
    }
    let col = snap_buffer_col(&*app.buffer, current.row, col)?;
    Ok(Cursor {
        row: current.row,
        col,
    })
}

fn word_right(app: &super::App) -> io::Result<Cursor> {
    let current = app.buffer.cursor();
    let line_len = app.buffer.line_char_count(current.row).unwrap_or(0);
    let mut col = current.col.min(line_len);
    if col == line_len {
        let last = app.buffer.line_count().saturating_sub(1);
        return Ok(if current.row < last {
            Cursor {
                row: current.row + 1,
                col: 0,
            }
        } else {
            current
        });
    }

    let first = line_window(&*app.buffer, current.row, col, 1)?
        .chars()
        .next();
    let Some(first) = first else {
        return Ok(current);
    };
    let starts_in_whitespace = first.is_whitespace();
    let class = word_class(first);
    let mut reached_whitespace = false;
    'chunks: while col < line_len {
        let take = WORD_WINDOW.min(line_len - col);
        let text = line_window(&*app.buffer, current.row, col, take)?;
        for ch in text.chars() {
            if (starts_in_whitespace || reached_whitespace) && !ch.is_whitespace() {
                break 'chunks;
            } else if word_class(ch) != class {
                if ch.is_whitespace() {
                    reached_whitespace = true;
                } else {
                    break 'chunks;
                }
            }
            col += 1;
        }
        if take == 0 {
            break;
        }
    }
    let col = ceil_buffer_col(&*app.buffer, current.row, col)?;
    Ok(Cursor {
        row: current.row,
        col,
    })
}

fn ceil_buffer_col(buffer: &dyn Buffer, row: usize, col: usize) -> io::Result<usize> {
    let floor = snap_buffer_col(buffer, row, col)?;
    if floor == col {
        return Ok(floor);
    }
    let line_len = buffer.line_char_count(row).unwrap_or(0);
    let remaining = line_len.saturating_sub(floor);
    let mut width = GRAPHEME_WINDOW.min(remaining);
    loop {
        let text = line_window(buffer, row, floor, width)?;
        let next = text_layout::next_grapheme_col(&text, 0);
        if next < text.chars().count() || width == remaining {
            return Ok(floor.saturating_add(next));
        }
        width = width.saturating_mul(2).min(remaining);
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
        assert_eq!(app.buffer.cursor().row, 3);
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
    fn ordinary_movement_and_deletion_treat_a_grapheme_as_one_unit() {
        let mut app = app("a\u{301}猫x");
        let mut out = Vec::new();

        move_grapheme(&mut app, true).unwrap();
        assert_eq!(app.buffer.cursor().col, 2);
        move_grapheme(&mut app, true).unwrap();
        assert_eq!(app.buffer.cursor().col, 3);
        move_grapheme(&mut app, false).unwrap();
        assert_eq!(app.buffer.cursor().col, 2);

        delete_grapheme(&mut app, &mut out, false).unwrap();
        assert_eq!(app.buffer.to_string(), "猫x");
        assert_eq!(app.buffer.cursor(), Cursor::default());
        app.buffer.undo();
        assert_eq!(app.buffer.to_string(), "a\u{301}猫x");
    }

    #[test]
    fn movement_keeps_zwj_emoji_combining_marks_and_tabs_on_scalar_boundaries() {
        let mut app = app("a\u{301}\t👩\u{200d}💻x");

        move_grapheme(&mut app, true).unwrap();
        assert_eq!(app.buffer.cursor().col, 2);
        move_grapheme(&mut app, true).unwrap();
        assert_eq!(app.buffer.cursor().col, 3);
        move_grapheme(&mut app, true).unwrap();
        assert_eq!(app.buffer.cursor().col, 6);
        move_grapheme(&mut app, false).unwrap();
        assert_eq!(app.buffer.cursor().col, 3);
    }

    #[test]
    fn crlf_normalization_keeps_word_and_grapheme_navigation_exact() {
        let mut app = app("a\u{301}\t👩\u{200d}💻 x\r\nnext");
        assert_eq!(app.buffer.line_count(), 2);

        for expected_col in [2, 3, 7] {
            let target = word_right(&app).unwrap();
            app.buffer.set_cursor(target);
            assert_eq!(
                app.buffer.cursor(),
                Cursor {
                    row: 0,
                    col: expected_col
                }
            );
        }
        let target = word_left(&app).unwrap();
        app.buffer.set_cursor(target);
        assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 3 });
        app.buffer.set_cursor(Cursor { row: 0, col: 8 });
        let target = word_right(&app).unwrap();
        assert_eq!(target, Cursor { row: 1, col: 0 });
    }

    #[test]
    fn word_movement_streams_across_long_lines_and_snaps_graphemes() {
        let word = "x".repeat(1024 * 1024);
        let mut app = app(&format!("{word}  a\u{301} 👩\u{200d}💻"));
        let mut out = Vec::new();

        handle_key(
            &mut app,
            &mut out,
            key(KeyCode::Right, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
        )
        .unwrap();
        assert_eq!(app.buffer.cursor().col, word.len() + 2);
        assert_eq!(
            app.selection.active().unwrap().ordered(),
            (
                Cursor::default(),
                Cursor {
                    row: 0,
                    col: word.len() + 2
                }
            )
        );
        handle_key(
            &mut app,
            &mut out,
            key(KeyCode::Right, KeyModifiers::CONTROL),
        )
        .unwrap();
        assert_eq!(app.buffer.cursor().col, word.len() + 4);
        handle_key(
            &mut app,
            &mut out,
            key(KeyCode::Left, KeyModifiers::CONTROL),
        )
        .unwrap();
        assert_eq!(app.buffer.cursor().col, word.len() + 2);
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

    #[test]
    fn plain_backspace_event_deletes_one_unicode_grapheme() {
        let mut app = app("one a\u{301}");
        let mut out = Vec::new();
        app.buffer.set_cursor(Cursor { row: 0, col: 6 });

        app.handle_key_with(&mut out, key(KeyCode::Backspace, KeyModifiers::NONE))
            .unwrap();

        assert_eq!(app.buffer.to_string(), "one ");
        app.buffer.undo();
        assert_eq!(app.buffer.to_string(), "one a\u{301}");
    }

    #[test]
    fn repeated_grapheme_deletions_share_one_undo_run() {
        let source = "a\u{301}b\u{301}c";
        let mut backspace = app(source);
        let mut out = Vec::new();
        backspace.buffer.set_cursor(Cursor { row: 0, col: 5 });

        for _ in 0..2 {
            backspace
                .handle_key_with(&mut out, key(KeyCode::Backspace, KeyModifiers::NONE))
                .unwrap();
        }
        assert_eq!(backspace.buffer.to_string(), "a\u{301}");
        backspace.buffer.undo();
        assert_eq!(backspace.buffer.to_string(), source);

        let mut forward = app(source);
        for _ in 0..2 {
            forward
                .handle_key_with(&mut out, key(KeyCode::Delete, KeyModifiers::NONE))
                .unwrap();
        }
        assert_eq!(forward.buffer.to_string(), "c");
        forward.buffer.undo();
        assert_eq!(forward.buffer.to_string(), source);
    }
}
