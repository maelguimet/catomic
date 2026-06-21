//! Piece table core implementing Buffer (Phase 1B).
//!
//! Purpose: fast, correct text storage behind the stable Buffer trait using
//!          a piece table + line index.
//! Owns: the two source strings, piece list, LineIndex, cursor + byte offset.
//! Must not: implement undo (1C), touch LLM/project, or mutate outside Buffer.
//! Invariants:
//! - Piece ranges are always on UTF-8 boundaries.
//! - LineIndex is consistent with current pieces after every edit (rebuild first).
//! - cursor_byte_offset == logical byte position of cursor.
//! Phase: 1B (line index, query optimization, coalescing; no undo).

mod edit;
mod index;
mod query;
mod types;

use std::borrow::Cow;

use crate::buffer::{Buffer, Cursor, LineView};

pub use types::PieceTable;
use types::{LineIndex, Piece, Source};

impl PieceTable {
    pub fn new() -> Self {
        let pieces = vec![Piece {
            source: Source::Original,
            start: 0,
            len: 0,
        }];
        let index = LineIndex::rebuild_from_pieces("", "", &pieces);
        Self {
            original: String::new(),
            add: String::new(),
            pieces,
            index,
            cursor: Cursor { row: 0, col: 0 },
            cursor_byte_offset: 0,
        }
    }

    pub fn from_text(text: &str) -> Self {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let (original, pieces) = if normalized.is_empty() {
            (
                String::new(),
                vec![Piece {
                    source: Source::Original,
                    start: 0,
                    len: 0,
                }],
            )
        } else {
            let len = normalized.len();
            (
                normalized,
                vec![Piece {
                    source: Source::Original,
                    start: 0,
                    len,
                }],
            )
        };
        let index = LineIndex::rebuild_from_pieces(&original, "", &pieces);
        Self {
            original,
            add: String::new(),
            pieces,
            index,
            cursor: Cursor { row: 0, col: 0 },
            cursor_byte_offset: 0,
        }
    }

    /// Rebuild index from current pieces. Call after every structural edit (1B bridge).
    pub(crate) fn rebuild_index(&mut self) {
        self.index = LineIndex::rebuild_from_pieces(&self.original, &self.add, &self.pieces);
        // Re-sync byte offset from current (row,col) using the fresh index
        self.cursor_byte_offset = self.byte_offset_at(self.cursor.row, self.cursor.col);
    }

    /// Coalesce adjacent same-source contiguous pieces. Call after edit.
    pub(crate) fn coalesce(&mut self) {
        if self.pieces.len() < 2 {
            return;
        }
        let mut i = 0;
        while i + 1 < self.pieces.len() {
            let p1 = &self.pieces[i];
            let p2 = &self.pieces[i + 1];
            if p1.source == p2.source && p1.start + p1.len == p2.start {
                let merged = Piece {
                    source: p1.source,
                    start: p1.start,
                    len: p1.len + p2.len,
                };
                self.pieces[i] = merged;
                self.pieces.remove(i + 1);
                // stay at i to check further merges
            } else {
                i += 1;
            }
        }
    }
}

impl Buffer for PieceTable {
    fn line_count(&self) -> usize {
        self.index.line_count()
    }

    fn line(&self, row: usize) -> Option<Cow<'_, str>> {
        let n = self.index.line_count();
        if row >= n {
            return None;
        }
        let start = self.index.line_start_byte(row);
        let end = self.index.line_end_byte(row);
        let content = self.slice_to_string(start, end);
        Some(Cow::Owned(content))
    }

    fn visible_lines(&self, start: usize, height: usize) -> Vec<LineView> {
        let n = self.index.line_count();
        let end = (start + height).min(n);
        (start..end)
            .map(|r| {
                let s = self.index.line_start_byte(r);
                let e = self.index.line_end_byte(r);
                LineView {
                    content: self.slice_to_string(s, e),
                }
            })
            .collect()
    }

    fn cursor(&self) -> Cursor {
        self.cursor
    }

    fn to_string(&self) -> String {
        // to_string / save is not a hot per-keypath; slice is fine too.
        self.slice_to_string(0, self.index.total_bytes)
    }

    fn lines(&self) -> Vec<String> {
        let n = self.index.line_count();
        (0..n)
            .map(|r| {
                let s = self.index.line_start_byte(r);
                let e = self.index.line_end_byte(r);
                self.slice_to_string(s, e)
            })
            .collect()
    }

    fn insert_char(&mut self, ch: char) {
        self.insert_at_cursor(ch);
        self.coalesce();
        self.rebuild_index();
    }

    fn insert_newline(&mut self) {
        self.insert_at_cursor('\n');
        self.coalesce();
        self.rebuild_index();
    }

    fn delete_back(&mut self) {
        if self.cursor.col > 0 {
            let end_b = self.byte_offset_at(self.cursor.row, self.cursor.col);
            let start_b = self.byte_offset_at(self.cursor.row, self.cursor.col - 1);
            self.delete_byte_range(start_b, end_b);
            self.cursor.col -= 1;
            self.cursor_byte_offset = start_b;
        } else if self.cursor.row > 0 {
            let nl_pos = self.byte_offset_at(self.cursor.row, 0);
            if nl_pos > 0 {
                let prev_len = self.current_line_char_len(self.cursor.row - 1);
                self.delete_byte_range(nl_pos - 1, nl_pos);
                self.cursor.row -= 1;
                self.cursor.col = prev_len;
                self.cursor_byte_offset = self.byte_offset_at(self.cursor.row, self.cursor.col);
            }
        }
        self.coalesce();
        self.rebuild_index();
    }

    fn delete_forward(&mut self) {
        let len = self.current_line_char_len(self.cursor.row);
        if self.cursor.col < len {
            let start_b = self.byte_offset_at(self.cursor.row, self.cursor.col);
            let end_b = self.byte_offset_at(self.cursor.row, self.cursor.col + 1);
            self.delete_byte_range(start_b, end_b);
            // col unchanged, byte already adjusted in delete
        } else if self.cursor.row + 1 < self.line_count() {
            let next_start = self.byte_offset_at(self.cursor.row + 1, 0);
            if next_start > 0 {
                let nl_pos = next_start - 1;
                self.delete_byte_range(nl_pos, nl_pos + 1);
            }
        }
        self.coalesce();
        self.rebuild_index();
    }

    fn move_left(&mut self) {
        self.move_left_internal();
        self.cursor_byte_offset = self.byte_offset_at(self.cursor.row, self.cursor.col);
    }

    fn move_right(&mut self) {
        self.move_right_internal();
        self.cursor_byte_offset = self.byte_offset_at(self.cursor.row, self.cursor.col);
    }

    fn move_up(&mut self) {
        self.move_up_internal();
        self.cursor_byte_offset = self.byte_offset_at(self.cursor.row, self.cursor.col);
    }

    fn move_down(&mut self) {
        self.move_down_internal();
        self.cursor_byte_offset = self.byte_offset_at(self.cursor.row, self.cursor.col);
    }
}
