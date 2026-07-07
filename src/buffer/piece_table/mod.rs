//! Piece table core implementing Buffer (Phases 1B-1C).
//!
//! Purpose: correct + optimized text storage (piece table + line index) behind Buffer.
//! Owns: original/add, pieces, LineIndex, cursor + byte offset cache, undo/redo tx stack.
//! Must not: LLM/project/config, UI expansion.
//! Invariants:
//! - Pieces UTF-8 char-boundary safe, cover logical doc.
//! - index consistent after edit (rebuild bridge first; incremental later).
//! - cursor_byte_offset always matches (row, col) position.
//! - Queries (line/visible) use slice + index, no full materialization.
//! Phase: 1B-1C.

mod construct;
mod edit;
mod query;
pub(crate) mod types;

use std::borrow::Cow;

use crate::buffer::line_index::LineIndex;
use crate::buffer::{Buffer, Cursor, LineView};

use crate::buffer::undo::{CursorState, PieceEdit, Transaction};
pub use types::PieceTable;
use types::{OriginalBacking, Piece, Source};

impl PieceTable {
    /// Rebuild index from current pieces. Call after every structural edit (1B bridge).
    /// Searches pieces for \n byte positions; kept here because it needs Piece/Source.
    /// Common index builder (used by ctors and rebuild). Avoids depending on
    /// external rebuild in LineIndex (which would need Piece/Source types).
    fn build_index(original: &OriginalBacking, add: &str, pieces: &[Piece]) -> LineIndex {
        let mut line_starts = vec![0usize];
        let mut acc: usize = 0;
        for p in pieces {
            match p.source {
                Source::Original => {
                    original.for_each_newline(p.start..p.start + p.len, |source_i| {
                        line_starts.push(acc + (source_i - p.start) + 1);
                    });
                }
                Source::Add => {
                    let text = &add[p.start..p.start + p.len];
                    for (i, _) in text.match_indices('\n') {
                        line_starts.push(acc + i + 1);
                    }
                }
            }
            acc += p.len;
        }
        if line_starts.is_empty() {
            line_starts.push(0);
        }
        LineIndex {
            line_starts,
            total_bytes: acc,
        }
    }

    pub(crate) fn rebuild_index(&mut self) {
        self.index = Self::build_index(&self.original, &self.add, &self.pieces);
        // Re-sync byte offset from current (row,col) using the fresh index
        self.cursor_byte_offset = self.byte_offset_at(self.cursor.row, self.cursor.col);
    }

    /// Coalesce adjacent same-source contiguous pieces. Call after edit.
    /// Rule: if same Source and p1.start + p1.len == p2.start then merge.
    pub(crate) fn coalesce(&mut self) {
        if self.pieces.len() < 2 {
            self.sync_piece_starts();
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
        self.sync_piece_starts();
    }

    /// Keep piece_starts parallel to pieces after any structural mutation.
    fn sync_piece_starts(&mut self) {
        self.piece_starts.clear();
        let mut acc = 0usize;
        for p in &self.pieces {
            self.piece_starts.push(acc);
            acc += p.len;
        }
    }

    fn capture_cursor_state(&self) -> CursorState {
        CursorState {
            cursor: self.cursor,
            byte_offset: self.cursor_byte_offset,
        }
    }

    #[cfg(test)]
    pub(crate) fn pieces_len(&self) -> usize {
        self.pieces.len()
    }

    /// Incremental update for index when edit does not add/remove a '\n'.
    /// Shifts subsequent line starts and total_bytes.
    fn adjust_index_for_simple_delta(&mut self, at_byte: usize, delta: isize) {
        if delta == 0 {
            return;
        }
        let row = self.index.row_for_byte(at_byte);
        let dpos = delta.unsigned_abs();
        if delta > 0 {
            for ls in &mut self.index.line_starts[(row + 1)..] {
                *ls += dpos;
            }
            self.index.total_bytes += dpos;
        } else {
            for ls in &mut self.index.line_starts[(row + 1)..] {
                *ls -= dpos;
            }
            self.index.total_bytes -= dpos;
        }
    }

    /// Incremental line index update for inserting a single '\n' at at_byte.
    /// Inserts the new line boundary and shifts tail. Does not rescan text.
    fn adjust_index_for_newline_insert(&mut self, at_byte: usize) {
        let row = self.index.row_for_byte(at_byte);
        // Shift subsequent line starts for the added byte.
        for ls in &mut self.index.line_starts[(row + 1)..] {
            *ls += 1;
        }
        let new_line_start = at_byte + 1;
        self.index.line_starts.insert(row + 1, new_line_start);
        self.index.total_bytes += 1;
    }

    /// Incremental line index update for deleting a '\n' at nl_pos.
    /// Removes the following line boundary and shifts tail. Does not rescan.
    fn adjust_index_for_newline_delete(&mut self, nl_pos: usize) {
        let boundary = nl_pos + 1;
        // Find index of the exact boundary being removed.
        let mut idx: Option<usize> = None;
        for (i, &ls) in self.index.line_starts.iter().enumerate().skip(1) {
            if ls == boundary {
                idx = Some(i);
                break;
            }
        }
        let idx = match idx {
            Some(i) => i,
            None => {
                // Defensive: fall back (should not happen in normal nl join).
                self.rebuild_index();
                return;
            }
        };
        // Shift the boundary and tail down by 1 for removed byte.
        for ls in &mut self.index.line_starts[idx..] {
            *ls = ls.saturating_sub(1);
        }
        self.index.line_starts.remove(idx);
        self.index.total_bytes = self.index.total_bytes.saturating_sub(1);
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
        let was_nl = ch == '\n';
        let before = self.capture_cursor_state();
        let at = self.cursor_byte_offset;
        let inserted = self.insert_at_cursor(ch);
        self.coalesce();
        if was_nl {
            self.rebuild_index();
        } else {
            let delta = ch.len_utf8() as isize;
            self.adjust_index_for_simple_delta(at, delta);
            // cursor_byte_offset was already advanced inside insert_at_cursor
        }
        if self.recording && !inserted.is_empty() {
            let after = self.capture_cursor_state();
            let tx = Transaction {
                before,
                after,
                edits: vec![PieceEdit::Insert {
                    at,
                    pieces: inserted,
                }],
                id: 0,
            };
            self.undo_stack.record(tx);
        }
    }

    fn insert_newline(&mut self) {
        let before = self.capture_cursor_state();
        let at = self.cursor_byte_offset;
        let inserted = self.insert_at_cursor('\n');
        self.coalesce();
        // Use incremental for the added boundary (no full text scan).
        self.adjust_index_for_newline_insert(at);
        if self.recording && !inserted.is_empty() {
            let after = self.capture_cursor_state();
            let tx = Transaction {
                before,
                after,
                edits: vec![PieceEdit::Insert {
                    at,
                    pieces: inserted,
                }],
                id: 0,
            };
            self.undo_stack.record(tx);
        }
    }

    fn delete_back(&mut self) {
        if self.cursor.col > 0 {
            let end_b = self.byte_offset_at(self.cursor.row, self.cursor.col);
            let start_b = self.byte_offset_at(self.cursor.row, self.cursor.col - 1);
            let delta = -((end_b - start_b) as isize);
            let before = self.capture_cursor_state();
            let removed = self.delete_byte_range(start_b, end_b);
            self.cursor.col -= 1;
            self.cursor_byte_offset = start_b;
            self.coalesce();
            // within-line non-nl char delete: incremental shift
            self.adjust_index_for_simple_delta(start_b, delta);
            if self.recording && !removed.is_empty() {
                let after = self.capture_cursor_state();
                let tx = Transaction {
                    before,
                    after,
                    edits: vec![PieceEdit::Delete {
                        at: start_b,
                        pieces: removed,
                    }],
                    id: 0,
                };
                self.undo_stack.record(tx);
            }
        } else if self.cursor.row > 0 {
            let nl_pos = self.byte_offset_at(self.cursor.row, 0);
            if nl_pos > 0 {
                let prev_len = self.current_line_char_len(self.cursor.row - 1);
                let before = self.capture_cursor_state();
                let removed = self.delete_byte_range(nl_pos - 1, nl_pos);
                self.cursor.row -= 1;
                self.cursor.col = prev_len;
                self.cursor_byte_offset = self.byte_offset_at(self.cursor.row, self.cursor.col);
                self.coalesce();
                self.adjust_index_for_newline_delete(nl_pos - 1);
                if self.recording && !removed.is_empty() {
                    let after = self.capture_cursor_state();
                    let tx = Transaction {
                        before,
                        after,
                        edits: vec![PieceEdit::Delete {
                            at: nl_pos - 1,
                            pieces: removed,
                        }],
                        id: 0,
                    };
                    self.undo_stack.record(tx);
                }
            } else {
                self.coalesce();
            }
        } else {
            self.coalesce();
            // no-op
        }
    }

    fn delete_forward(&mut self) {
        let len = self.current_line_char_len(self.cursor.row);
        if self.cursor.col < len {
            let start_b = self.byte_offset_at(self.cursor.row, self.cursor.col);
            let end_b = self.byte_offset_at(self.cursor.row, self.cursor.col + 1);
            let delta = -((end_b - start_b) as isize);
            let before = self.capture_cursor_state();
            let removed = self.delete_byte_range(start_b, end_b);
            self.coalesce();
            // within-line non-nl char delete
            self.adjust_index_for_simple_delta(start_b, delta);
            if self.recording && !removed.is_empty() {
                let after = self.capture_cursor_state();
                let tx = Transaction {
                    before,
                    after,
                    edits: vec![PieceEdit::Delete {
                        at: start_b,
                        pieces: removed,
                    }],
                    id: 0,
                };
                self.undo_stack.record(tx);
            }
            // col unchanged
        } else if self.cursor.row + 1 < self.line_count() {
            let next_start = self.byte_offset_at(self.cursor.row + 1, 0);
            if next_start > 0 {
                let nl_pos = next_start - 1;
                let before = self.capture_cursor_state();
                let removed = self.delete_byte_range(nl_pos, nl_pos + 1);
                self.coalesce();
                self.adjust_index_for_newline_delete(nl_pos);
                if self.recording && !removed.is_empty() {
                    let after = self.capture_cursor_state();
                    let tx = Transaction {
                        before,
                        after,
                        edits: vec![PieceEdit::Delete {
                            at: nl_pos,
                            pieces: removed,
                        }],
                        id: 0,
                    };
                    self.undo_stack.record(tx);
                }
            } else {
                self.coalesce();
            }
        } else {
            self.coalesce();
        }
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

    fn undo(&mut self) {
        if let Some(tx) = self.undo_stack.pop_undo() {
            let was_recording = self.recording;
            self.recording = false;

            // Apply inverses (LIFO single tx; edits reversed for safety if multi).
            for e in tx.edits.iter().rev() {
                match e {
                    PieceEdit::Insert { at, pieces } => {
                        let len: usize = pieces.iter().map(|p| p.len).sum();
                        self.delete_byte_range(*at, *at + len);
                    }
                    PieceEdit::Delete { at, pieces } => {
                        self.insert_pieces_at(*at, pieces);
                    }
                }
            }
            self.coalesce();
            self.rebuild_index();

            self.cursor = tx.before.cursor;
            self.cursor_byte_offset = tx.before.byte_offset;

            self.undo_stack.push_redo(tx);
            self.recording = was_recording;
        }
    }

    fn redo(&mut self) {
        if let Some(tx) = self.undo_stack.pop_redo() {
            let was_recording = self.recording;
            self.recording = false;

            for e in &tx.edits {
                match e {
                    PieceEdit::Insert { at, pieces } => {
                        self.insert_pieces_at(*at, pieces);
                    }
                    PieceEdit::Delete { at, pieces } => {
                        let len: usize = pieces.iter().map(|p| p.len).sum();
                        self.delete_byte_range(*at, *at + len);
                    }
                }
            }
            self.coalesce();
            self.rebuild_index();

            self.cursor = tx.after.cursor;
            self.cursor_byte_offset = tx.after.byte_offset;

            self.undo_stack.push_undo(tx);
            self.recording = was_recording;
        }
    }

    fn edit_history_position(&self) -> u64 {
        self.undo_stack.current_history_position()
    }
}
