//! Purpose: provide configured indentation, auto-indent newline, and line unindent.
//! Owns: Tab/Shift+Tab edit ranges and inherited/block indentation calculation.
//! Must not: parse whole files, infer language services, save, or run background work.
//! Invariants: each user action is one replacement transaction; selected text is retained.

use std::io::{self, Write};

use crate::buffer::Cursor;

pub(crate) fn insert_newline(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let (start, end) = selected_or_cursor(app);
    let line = app.buffer.line(start.row).unwrap_or_default();
    let prefix: String = line.chars().take_while(|ch| ch.is_whitespace()).collect();
    let before: String = line.chars().take(start.col).collect();
    let width = tab_width(app);
    let opens_block = before
        .trim_end()
        .chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, '{' | '[' | '(' | ':'));
    let extra = if opens_block {
        " ".repeat(width)
    } else {
        String::new()
    };
    let text = format!("\n{prefix}{extra}");
    if start == end && text == "\n" {
        app.buffer.insert_newline();
    } else {
        app.buffer.replace_range(start, end, &text)?;
    }
    super::input::finish_content_edit(app, out)
}

pub(crate) fn handle_tab(
    app: &mut super::App,
    out: &mut dyn Write,
    unindent: bool,
) -> io::Result<()> {
    if let Some(selection) = app.selection.active() {
        let (start, end) = selection.ordered();
        let last_row = if end.col == 0 && end.row > start.row {
            end.row - 1
        } else {
            end.row
        };
        edit_lines(app, out, start.row, last_row, unindent)
    } else if unindent {
        let row = app.buffer.cursor().row;
        edit_lines(app, out, row, row, true)
    } else {
        insert_to_tab_stop(app, out)
    }
}

fn insert_to_tab_stop(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let cursor = app.buffer.cursor();
    let width = tab_width(app);
    let spaces = width - cursor.col % width;
    app.buffer
        .replace_range(cursor, cursor, &" ".repeat(spaces))?;
    super::input::finish_content_edit(app, out)
}

fn edit_lines(
    app: &mut super::App,
    out: &mut dyn Write,
    first_row: usize,
    last_row: usize,
    unindent: bool,
) -> io::Result<()> {
    let width = tab_width(app);
    let old_cursor = app.buffer.cursor();
    let mut changed_on_cursor_row = 0;
    let mut lines = Vec::new();
    for row in first_row..=last_row {
        let line = app.buffer.line(row).unwrap_or_default();
        if unindent {
            let removed = removable_indent(&line, width);
            if row == old_cursor.row {
                changed_on_cursor_row = removed;
            }
            lines.push(line.chars().skip(removed).collect::<String>());
        } else {
            if row == old_cursor.row {
                changed_on_cursor_row = width;
            }
            lines.push(format!("{}{}", " ".repeat(width), line));
        }
    }
    if unindent
        && lines.iter().enumerate().all(|(offset, edited)| {
            app.buffer
                .line(first_row + offset)
                .is_some_and(|line| line == edited.as_str())
        })
    {
        return super::input::finish_content_edit(app, out);
    }
    let end = Cursor {
        row: last_row,
        col: app.buffer.line_char_count(last_row).unwrap_or(0),
    };
    app.buffer.replace_range(
        Cursor {
            row: first_row,
            col: 0,
        },
        end,
        &lines.join("\n"),
    )?;
    let col = if unindent {
        old_cursor.col.saturating_sub(changed_on_cursor_row)
    } else {
        old_cursor.col.saturating_add(changed_on_cursor_row)
    };
    app.buffer.set_cursor(Cursor {
        row: old_cursor.row,
        col,
    });
    super::input::finish_content_edit(app, out)
}

fn removable_indent(line: &str, width: usize) -> usize {
    if line.starts_with('\t') {
        return 1;
    }
    line.chars().take(width).take_while(|ch| *ch == ' ').count()
}

fn selected_or_cursor(app: &super::App) -> (Cursor, Cursor) {
    app.selection
        .active()
        .map(|selection| selection.ordered())
        .unwrap_or_else(|| {
            let cursor = app.buffer.cursor();
            (cursor, cursor)
        })
}

fn tab_width(app: &super::App) -> usize {
    app.editor_config
        .tab_size_for_path(app.file.path.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(text: &str, cursor: Cursor) -> super::super::App {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text(text));
        app.buffer.set_cursor(cursor);
        app
    }

    #[test]
    fn newline_inherits_indent_and_adds_one_level_after_block_opener() {
        let mut app = app("  if ready {", Cursor { row: 0, col: 12 });
        let mut out = Vec::new();
        insert_newline(&mut app, &mut out).unwrap();

        assert_eq!(app.buffer.to_string(), "  if ready {\n      ");
        app.buffer.undo();
        assert_eq!(app.buffer.to_string(), "  if ready {");
    }

    #[test]
    fn tab_indents_selected_lines_without_replacing_them() {
        let mut app = app("one\ntwo", Cursor { row: 1, col: 3 });
        let mut out = Vec::new();
        app.buffer.set_cursor(Cursor { row: 0, col: 0 });
        super::super::selection::move_to(&mut app, &mut out, Cursor { row: 1, col: 3 }, true)
            .unwrap();
        handle_tab(&mut app, &mut out, false).unwrap();

        assert_eq!(app.buffer.to_string(), "    one\n    two");
        app.buffer.undo();
        assert_eq!(app.buffer.to_string(), "one\ntwo");
    }

    #[test]
    fn shift_tab_unindents_selected_lines_as_one_edit() {
        let mut app = app("    one\n  two", Cursor { row: 1, col: 5 });
        let mut out = Vec::new();
        app.buffer.set_cursor(Cursor { row: 0, col: 0 });
        super::super::selection::move_to(&mut app, &mut out, Cursor { row: 1, col: 5 }, true)
            .unwrap();
        handle_tab(&mut app, &mut out, true).unwrap();

        assert_eq!(app.buffer.to_string(), "one\ntwo");
        app.buffer.undo();
        assert_eq!(app.buffer.to_string(), "    one\n  two");
    }
}
