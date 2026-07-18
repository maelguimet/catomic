//! Purpose: implement the stable Buffer contract for PieceTable.
//! Owns: PieceTable query, mutation, movement, history, and streaming adapters.
//! Must not: own storage layout, file opening, App policy, rendering, Project, or LLM work.
//! Invariants: all edits preserve piece/index/cursor consistency; file-backed
//!   render and save paths propagate descriptor errors through fallible seams.
//! Phase: 2-bj PieceTable size hygiene and bounded file-backed queries.

use std::borrow::Cow;
use std::io::{self, Write};

use crate::buffer::undo::{PieceEdit, Transaction};
use crate::buffer::{Buffer, Cursor, LineView, TextEdit};

use super::types::{Piece, PieceTable, Source};

impl Buffer for PieceTable {
    fn line_count(&self) -> usize {
        self.index.line_count()
    }

    fn line(&self, row: usize) -> Option<Cow<'_, str>> {
        if row >= self.index.line_count() {
            return None;
        }
        let start = self.index.line_start_byte(row);
        let end = self.index.line_end_byte(row);
        Some(Cow::Owned(self.slice_to_string(start, end)))
    }

    fn visible_lines(&self, start: usize, height: usize) -> Vec<LineView> {
        let end = (start + height).min(self.index.line_count());
        (start..end)
            .map(|row| LineView {
                content: self.slice_to_string(
                    self.index.line_start_byte(row),
                    self.index.line_end_byte(row),
                ),
            })
            .collect()
    }

    fn visible_lines_window(
        &self,
        start: usize,
        height: usize,
        start_col: usize,
        width: usize,
    ) -> Vec<LineView> {
        self.try_visible_lines_window(start, height, start_col, width)
            .unwrap_or_default()
    }

    fn try_visible_lines_window(
        &self,
        start: usize,
        height: usize,
        start_col: usize,
        width: usize,
    ) -> io::Result<Vec<LineView>> {
        let end = (start + height).min(self.index.line_count());
        (start..end)
            .map(|row| {
                self.try_window_to_string(
                    self.index.line_start_byte(row),
                    self.index.line_end_byte(row),
                    start_col,
                    width,
                )
                .map(|content| LineView { content })
            })
            .collect()
    }

    fn line_char_count(&self, row: usize) -> Option<usize> {
        if row >= self.index.line_count() {
            return None;
        }
        self.try_char_count(
            self.index.line_start_byte(row),
            self.index.line_end_byte(row),
        )
        .ok()
    }

    fn cursor(&self) -> Cursor {
        self.cursor
    }

    fn logical_byte_len(&self) -> Option<usize> {
        Some(self.index.total_bytes)
    }

    fn set_cursor(&mut self, cursor: Cursor) {
        let row = cursor.row.min(self.line_count().saturating_sub(1));
        let col = cursor.col.min(self.current_line_char_len(row));
        self.cursor = Cursor { row, col };
        self.sync_cursor_byte_offset();
    }

    fn text_range(&self, start: Cursor, end: Cursor) -> io::Result<String> {
        let (start, end) = self.clamped_ordered_range(start, end);
        self.try_slice_to_string(
            self.byte_offset_at(start.row, start.col),
            self.byte_offset_at(end.row, end.col),
        )
    }

    fn replace_range(&mut self, start: Cursor, end: Cursor, text: &str) -> io::Result<bool> {
        let (start, end) = self.clamped_ordered_range(start, end);
        let start_byte = self.byte_offset_at(start.row, start.col);
        let end_byte = self.byte_offset_at(end.row, end.col);
        if start_byte == end_byte && text.is_empty() {
            return Ok(false);
        }

        let before = self.capture_cursor_state();
        let (removed, inserted) = self.splice_replacement(start_byte, end_byte, text);
        self.coalesce();
        self.rebuild_index();
        self.cursor = cursor_after_text(start, text);
        self.cursor_byte_offset = start_byte + text.len();
        self.record_replacement(before, start_byte, removed, inserted);
        Ok(true)
    }

    fn replace_ranges(&mut self, ranges: &[(Cursor, Cursor)], text: &str) -> io::Result<usize> {
        let before = self.capture_cursor_state();
        let mut edits = Vec::with_capacity(ranges.len().saturating_mul(2));
        let mut changed = 0;
        for &(start, end) in ranges {
            let (start, end) = self.clamped_ordered_range(start, end);
            let start_byte = self.byte_offset_at(start.row, start.col);
            let end_byte = self.byte_offset_at(end.row, end.col);
            if start_byte == end_byte && text.is_empty() {
                continue;
            }
            let (removed, inserted) = self.splice_replacement(start_byte, end_byte, text);
            if !removed.is_empty() {
                edits.push(PieceEdit::Delete {
                    at: start_byte,
                    pieces: removed,
                });
            }
            if !inserted.is_empty() {
                edits.push(PieceEdit::Insert {
                    at: start_byte,
                    pieces: inserted,
                });
            }
            self.coalesce();
            self.rebuild_index();
            self.cursor = cursor_after_text(start, text);
            self.cursor_byte_offset = start_byte + text.len();
            changed += 1;
        }
        if self.recording {
            self.undo_stack.record(Transaction {
                before,
                after: self.capture_cursor_state(),
                edits,
                id: 0,
            });
        }
        Ok(changed)
    }

    fn replace_text_edits(&mut self, replacements: &[TextEdit]) -> io::Result<usize> {
        let mut previous_start = None;
        for replacement in replacements {
            let (start, end) = self.clamped_ordered_range(replacement.start, replacement.end);
            let start_byte = self.byte_offset_at(start.row, start.col);
            let end_byte = self.byte_offset_at(end.row, end.col);
            if previous_start.is_some_and(|previous| end_byte > previous) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "text edits must be non-overlapping and ordered from end to start",
                ));
            }
            previous_start = Some(start_byte);
        }
        let before = self.capture_cursor_state();
        let mut edits = Vec::with_capacity(replacements.len().saturating_mul(2));
        let mut changed = 0;
        for replacement in replacements {
            let (start, end) = self.clamped_ordered_range(replacement.start, replacement.end);
            let start_byte = self.byte_offset_at(start.row, start.col);
            let end_byte = self.byte_offset_at(end.row, end.col);
            if start_byte == end_byte && replacement.replacement.is_empty() {
                continue;
            }
            let (removed, inserted) =
                self.splice_replacement(start_byte, end_byte, &replacement.replacement);
            if !removed.is_empty() {
                edits.push(PieceEdit::Delete {
                    at: start_byte,
                    pieces: removed,
                });
            }
            if !inserted.is_empty() {
                edits.push(PieceEdit::Insert {
                    at: start_byte,
                    pieces: inserted,
                });
            }
            self.coalesce();
            self.rebuild_index();
            self.cursor = cursor_after_text(start, &replacement.replacement);
            self.cursor_byte_offset = start_byte + replacement.replacement.len();
            changed += 1;
        }
        if self.recording {
            self.undo_stack.record(Transaction {
                before,
                after: self.capture_cursor_state(),
                edits,
                id: 0,
            });
        }
        Ok(changed)
    }

    fn to_string(&self) -> String {
        self.slice_to_string(0, self.index.total_bytes)
    }

    fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        for piece in &self.pieces {
            let range = piece.start..piece.start + piece.len;
            match piece.source {
                Source::Original => self.original.write_slice(range, out)?,
                Source::Add => out.write_all(self.add[range].as_bytes())?,
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn lines(&self) -> Vec<String> {
        (0..self.index.line_count())
            .map(|row| {
                self.slice_to_string(
                    self.index.line_start_byte(row),
                    self.index.line_end_byte(row),
                )
            })
            .collect()
    }

    fn insert_char(&mut self, ch: char) {
        let before = self.capture_cursor_state();
        let at = self.cursor_byte_offset;
        let inserted = self.insert_at_cursor(ch);
        self.coalesce();
        if ch == '\n' {
            self.rebuild_index();
        } else {
            self.adjust_index_for_simple_delta(at, ch.len_utf8() as isize);
        }
        if self.recording && !inserted.is_empty() {
            let after = self.capture_cursor_state();
            self.undo_stack.record(Transaction {
                before,
                after,
                edits: vec![PieceEdit::Insert {
                    at,
                    pieces: inserted,
                }],
                id: 0,
            });
        }
    }

    fn insert_newline(&mut self) {
        let before = self.capture_cursor_state();
        let at = self.cursor_byte_offset;
        let inserted = self.insert_at_cursor('\n');
        self.coalesce();
        self.adjust_index_for_newline_insert(at);
        if self.recording && !inserted.is_empty() {
            let after = self.capture_cursor_state();
            self.undo_stack.record(Transaction {
                before,
                after,
                edits: vec![PieceEdit::Insert {
                    at,
                    pieces: inserted,
                }],
                id: 0,
            });
        }
    }

    fn delete_back(&mut self) {
        if self.cursor.col > 0 {
            self.delete_previous_char();
        } else if self.cursor.row > 0 {
            self.join_with_previous_line();
        } else {
            self.coalesce();
        }
    }

    fn delete_forward(&mut self) {
        let len = self.current_line_char_len(self.cursor.row);
        if self.cursor.col < len {
            self.delete_next_char();
        } else if self.cursor.row + 1 < self.line_count() {
            self.join_with_next_line();
        } else {
            self.coalesce();
        }
    }

    fn move_left(&mut self) {
        self.move_left_internal();
        self.sync_cursor_byte_offset();
    }

    fn move_right(&mut self) {
        self.move_right_internal();
        self.sync_cursor_byte_offset();
    }

    fn move_up(&mut self) {
        self.move_up_internal();
        self.sync_cursor_byte_offset();
    }

    fn move_down(&mut self) {
        self.move_down_internal();
        self.sync_cursor_byte_offset();
    }

    fn undo(&mut self) {
        if let Some(tx) = self.undo_stack.pop_undo() {
            let was_recording = self.recording;
            self.recording = false;
            for edit in tx.edits.iter().rev() {
                self.apply_inverse_edit(edit);
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
            for edit in &tx.edits {
                self.apply_edit(edit);
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

fn cursor_after_text(start: Cursor, text: &str) -> Cursor {
    let newline_count = text.bytes().filter(|byte| *byte == b'\n').count();
    if newline_count == 0 {
        Cursor {
            row: start.row,
            col: start.col + text.chars().count(),
        }
    } else {
        Cursor {
            row: start.row + newline_count,
            col: text.rsplit('\n').next().unwrap_or_default().chars().count(),
        }
    }
}

impl PieceTable {
    fn clamped_ordered_range(&self, start: Cursor, end: Cursor) -> (Cursor, Cursor) {
        let clamp = |cursor: Cursor| {
            let row = cursor.row.min(self.line_count().saturating_sub(1));
            Cursor {
                row,
                col: cursor.col.min(self.current_line_char_len(row)),
            }
        };
        let start = clamp(start);
        let end = clamp(end);
        if (start.row, start.col) <= (end.row, end.col) {
            (start, end)
        } else {
            (end, start)
        }
    }

    fn splice_replacement(
        &mut self,
        start: usize,
        end: usize,
        text: &str,
    ) -> (Vec<Piece>, Vec<Piece>) {
        let removed = self.delete_byte_range(start, end);
        if text.is_empty() {
            return (removed, Vec::new());
        }
        let piece = Piece {
            source: Source::Add,
            start: self.add.len(),
            len: text.len(),
        };
        self.add.push_str(text);
        self.insert_pieces_at(start, std::slice::from_ref(&piece));
        (removed, vec![piece])
    }

    fn record_replacement(
        &mut self,
        before: crate::buffer::undo::CursorState,
        at: usize,
        removed: Vec<Piece>,
        inserted: Vec<Piece>,
    ) {
        if !self.recording {
            return;
        }
        let mut edits = Vec::with_capacity(2);
        if !removed.is_empty() {
            edits.push(PieceEdit::Delete {
                at,
                pieces: removed,
            });
        }
        if !inserted.is_empty() {
            edits.push(PieceEdit::Insert {
                at,
                pieces: inserted,
            });
        }
        self.undo_stack.record(Transaction {
            before,
            after: self.capture_cursor_state(),
            edits,
            id: 0,
        });
    }

    fn delete_previous_char(&mut self) {
        let end = self.byte_offset_at(self.cursor.row, self.cursor.col);
        let start = self.byte_offset_at(self.cursor.row, self.cursor.col - 1);
        let before = self.capture_cursor_state();
        let removed = self.delete_byte_range(start, end);
        self.cursor.col -= 1;
        self.cursor_byte_offset = start;
        self.coalesce();
        self.adjust_index_for_simple_delta(start, -((end - start) as isize));
        self.record_delete(before, start, removed);
    }

    fn delete_next_char(&mut self) {
        let start = self.byte_offset_at(self.cursor.row, self.cursor.col);
        let end = self.byte_offset_at(self.cursor.row, self.cursor.col + 1);
        let before = self.capture_cursor_state();
        let removed = self.delete_byte_range(start, end);
        self.coalesce();
        self.adjust_index_for_simple_delta(start, -((end - start) as isize));
        self.record_delete(before, start, removed);
    }

    fn join_with_previous_line(&mut self) {
        let next_start = self.byte_offset_at(self.cursor.row, 0);
        if next_start == 0 {
            self.coalesce();
            return;
        }
        let previous_len = self.current_line_char_len(self.cursor.row - 1);
        let before = self.capture_cursor_state();
        let removed = self.delete_byte_range(next_start - 1, next_start);
        self.cursor.row -= 1;
        self.cursor.col = previous_len;
        self.sync_cursor_byte_offset();
        self.coalesce();
        self.adjust_index_for_newline_delete(next_start - 1);
        self.record_delete(before, next_start - 1, removed);
    }

    fn join_with_next_line(&mut self) {
        let next_start = self.byte_offset_at(self.cursor.row + 1, 0);
        if next_start == 0 {
            self.coalesce();
            return;
        }
        let newline = next_start - 1;
        let before = self.capture_cursor_state();
        let removed = self.delete_byte_range(newline, next_start);
        self.coalesce();
        self.adjust_index_for_newline_delete(newline);
        self.record_delete(before, newline, removed);
    }

    fn record_delete(
        &mut self,
        before: crate::buffer::undo::CursorState,
        at: usize,
        pieces: Vec<super::types::Piece>,
    ) {
        if self.recording && !pieces.is_empty() {
            self.undo_stack.record(Transaction {
                before,
                after: self.capture_cursor_state(),
                edits: vec![PieceEdit::Delete { at, pieces }],
                id: 0,
            });
        }
    }

    fn sync_cursor_byte_offset(&mut self) {
        self.cursor_byte_offset = self.byte_offset_at(self.cursor.row, self.cursor.col);
    }

    fn apply_inverse_edit(&mut self, edit: &PieceEdit) {
        match edit {
            PieceEdit::Insert { at, pieces } => {
                let len = pieces.iter().map(|piece| piece.len).sum::<usize>();
                self.delete_byte_range(*at, *at + len);
            }
            PieceEdit::Delete { at, pieces } => self.insert_pieces_at(*at, pieces),
        }
    }

    fn apply_edit(&mut self, edit: &PieceEdit) {
        match edit {
            PieceEdit::Insert { at, pieces } => self.insert_pieces_at(*at, pieces),
            PieceEdit::Delete { at, pieces } => {
                let len = pieces.iter().map(|piece| piece.len).sum::<usize>();
                self.delete_byte_range(*at, *at + len);
            }
        }
    }
}
