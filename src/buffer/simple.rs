//! Phase 0 simple buffer implementation.
//!
//! `Vec<String>` backed. Good enough for the first goblin loop.
//! Will be replaced by piece table (see piece_table.rs).

use std::borrow::Cow;

use super::{Buffer, Cursor, LineView};

/// The dead-simple buffer used for Phase 0.
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

        let lines = if lines.is_empty() { vec![String::new()] } else { lines };

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

    fn to_string(&self) -> String {
        self.lines.join("\n")
    }

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
}
