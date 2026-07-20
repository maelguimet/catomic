//! Simple in-memory buffer implementation.
//!
//! `Vec<String>` backed and retained as the reference implementation for
//! PieceTable parity tests.

use std::borrow::Cow;

use super::{Buffer, Cursor, CursorContext, LineView};

/// The simple buffer used as the observable-behavior reference in parity tests.
#[derive(Clone, Debug, Default)]
pub struct SimpleBuffer {
    lines: Vec<String>,
    cursor: Cursor,
}

impl SimpleBuffer {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: Cursor { row: 0, col: 0 },
        }
    }

    pub fn from_text(text: &str) -> Self {
        // Normalize line endings (Phase 0 is Linux-first, saves use \n).
        // Use split('\n') (not .lines()) so a trailing '\n' produces a final
        // empty string entry. This preserves "file ends with newline" shape
        // across open + save.
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let lines: Vec<String> = if normalized.is_empty() {
            vec![String::new()]
        } else {
            normalized.split('\n').map(|l| l.to_string()).collect()
        };

        let lines = if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        };

        // Editor convention: from_text (open file) starts cursor at top-left,
        // same as new(). Previously placed at EOF; fixed before using
        // SimpleBuffer as oracle for PieceTable parity tests.
        Self {
            lines,
            cursor: Cursor { row: 0, col: 0 },
        }
    }

    fn ensure_line(&mut self) {
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
    }

    fn current_line_len(&self) -> usize {
        self.lines
            .get(self.cursor.row)
            .map(|l| l.chars().count())
            .unwrap_or(0)
    }
}

impl Buffer for SimpleBuffer {
    fn line_count(&self) -> usize {
        self.lines.len().max(1)
    }

    fn line(&self, row: usize) -> Option<Cow<'_, str>> {
        self.lines.get(row).map(|s| Cow::Borrowed(s.as_str()))
    }

    fn visible_lines(&self, start: usize, height: usize) -> Vec<LineView> {
        let end = (start + height).min(self.line_count());
        (start..end)
            .map(|r| LineView {
                content: self.line(r).unwrap_or_default().to_string(),
            })
            .collect()
    }

    fn cursor(&self) -> Cursor {
        self.cursor
    }

    fn set_cursor(&mut self, cursor: Cursor) {
        let row = cursor.row.min(self.line_count().saturating_sub(1));
        let col = cursor
            .col
            .min(self.lines.get(row).map_or(0, |line| line.chars().count()));
        self.cursor = Cursor { row, col };
    }

    fn cursor_context(
        &self,
        max_before: usize,
        max_after: usize,
    ) -> std::io::Result<CursorContext> {
        Ok(CursorContext {
            before: context_before(&self.lines, self.cursor, max_before),
            after: context_after(&self.lines, self.cursor, max_after),
        })
    }

    fn to_string(&self) -> String {
        self.lines.join("\n")
    }

    #[cfg(test)]
    fn lines(&self) -> Vec<String> {
        self.lines.clone()
    }

    fn insert_char(&mut self, ch: char) {
        self.ensure_line();
        let line = &mut self.lines[self.cursor.row];
        let mut chars: Vec<char> = line.chars().collect();
        let col = self.cursor.col.min(chars.len());
        chars.insert(col, ch);
        *line = chars.into_iter().collect();
        self.cursor.col += 1;
    }

    fn insert_newline(&mut self) {
        self.ensure_line();
        let line = &mut self.lines[self.cursor.row];
        let mut chars: Vec<char> = line.chars().collect();
        let col = self.cursor.col.min(chars.len());

        let after: String = chars.drain(col..).collect();
        *line = chars.into_iter().collect();

        self.lines.insert(self.cursor.row + 1, after);
        self.cursor.row += 1;
        self.cursor.col = 0;
    }

    fn delete_back(&mut self) {
        if self.cursor.col > 0 {
            let line = &mut self.lines[self.cursor.row];
            let mut chars: Vec<char> = line.chars().collect();
            let col = self.cursor.col.min(chars.len());
            if col > 0 {
                chars.remove(col - 1);
                *line = chars.into_iter().collect();
                self.cursor.col -= 1;
            }
        } else if self.cursor.row > 0 {
            // Join with previous line
            let current = self.lines.remove(self.cursor.row);
            self.cursor.row -= 1;
            let prev = &mut self.lines[self.cursor.row];
            self.cursor.col = prev.chars().count();
            prev.push_str(&current);
        }
    }

    fn delete_forward(&mut self) {
        let len = self.current_line_len();
        if self.cursor.col < len {
            let line = &mut self.lines[self.cursor.row];
            let mut chars: Vec<char> = line.chars().collect();
            chars.remove(self.cursor.col);
            *line = chars.into_iter().collect();
        } else if self.cursor.row + 1 < self.lines.len() {
            // Join next line into current
            let next = self.lines.remove(self.cursor.row + 1);
            self.lines[self.cursor.row].push_str(&next);
        }
    }

    fn move_left(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.cursor.col = self.current_line_len();
        }
    }

    fn move_right(&mut self) {
        let len = self.current_line_len();
        if self.cursor.col < len {
            self.cursor.col += 1;
        } else if self.cursor.row + 1 < self.line_count() {
            self.cursor.row += 1;
            self.cursor.col = 0;
        }
    }

    fn move_up(&mut self) {
        if self.cursor.row > 0 {
            self.cursor.row -= 1;
            let len = self.current_line_len();
            self.cursor.col = self.cursor.col.min(len);
        }
    }

    fn move_down(&mut self) {
        if self.cursor.row + 1 < self.line_count() {
            self.cursor.row += 1;
            let len = self.current_line_len();
            self.cursor.col = self.cursor.col.min(len);
        }
    }

    fn undo(&mut self) {
        // SimpleBuffer has no undo history (Phase 1C is PieceTable only for now).
    }

    fn redo(&mut self) {
        // SimpleBuffer has no undo history (Phase 1C is PieceTable only for now).
    }

    fn edit_history_position(&self) -> u64 {
        // SimpleBuffer undo is a no-op stub; constant position is sufficient
        // for compilation and for any tests that construct it directly.
        0
    }
}

fn context_before(lines: &[String], cursor: Cursor, limit: usize) -> String {
    if lines.is_empty() || limit == 0 {
        return String::new();
    }
    let mut remaining = limit;
    let mut reversed = Vec::with_capacity(limit);
    let mut row = cursor.row.min(lines.len().saturating_sub(1));
    let line = &lines[row];
    let byte = scalar_byte_offset(line, cursor.col);
    append_reverse(&mut reversed, &line[..byte], &mut remaining);
    while remaining > 0 && row > 0 {
        reversed.push('\n');
        remaining -= 1;
        row -= 1;
        append_reverse(&mut reversed, &lines[row], &mut remaining);
    }
    reversed.reverse();
    reversed.into_iter().collect()
}

fn context_after(lines: &[String], cursor: Cursor, limit: usize) -> String {
    if lines.is_empty() || limit == 0 {
        return String::new();
    }
    let mut remaining = limit;
    let mut output = String::new();
    let mut row = cursor.row.min(lines.len().saturating_sub(1));
    let line = &lines[row];
    let byte = scalar_byte_offset(line, cursor.col);
    append_forward(&mut output, &line[byte..], &mut remaining);
    while remaining > 0 && row + 1 < lines.len() {
        output.push('\n');
        remaining -= 1;
        row += 1;
        append_forward(&mut output, &lines[row], &mut remaining);
    }
    output
}

fn scalar_byte_offset(line: &str, col: usize) -> usize {
    line.char_indices()
        .nth(col)
        .map_or(line.len(), |(byte, _)| byte)
}

fn append_reverse(output: &mut Vec<char>, text: &str, remaining: &mut usize) {
    let previous_len = output.len();
    output.extend(text.chars().rev().take(*remaining));
    *remaining = remaining.saturating_sub(output.len() - previous_len);
}

fn append_forward(output: &mut String, text: &str, remaining: &mut usize) {
    let taken: String = text.chars().take(*remaining).collect();
    *remaining = remaining.saturating_sub(taken.chars().count());
    output.push_str(&taken);
}
