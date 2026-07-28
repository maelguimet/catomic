//! Purpose: implement the stable Buffer contract for PieceTable.
//! Owns: PieceTable query, mutation, movement, history, and streaming adapters.
//! Must not: own storage layout, file opening, App policy, rendering, or external services.
//! Invariants: all edits preserve piece/index/cursor consistency; file-backed
//!   render and save paths propagate descriptor errors through fallible seams.

use std::borrow::Cow;
use std::io::{self, Write};

use crate::buffer::undo::{PieceEdit, Transaction, UndoRun};
use crate::buffer::{Buffer, Cursor, LineView};

use super::types::{Piece, PieceTable, Source};

struct SnapshotRange {
    start: Cursor,
    start_byte: usize,
    end_byte: usize,
}

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
        Some(self.try_slice_to_cow(start, end).unwrap_or_default())
    }

    fn visible_lines(&self, start: usize, height: usize) -> Vec<LineView<'_>> {
        let end = (start + height).min(self.index.line_count());
        (start..end)
            .map(|row| LineView {
                content: self
                    .try_slice_to_cow(
                        self.index.line_start_byte(row),
                        self.index.line_end_byte(row),
                    )
                    .unwrap_or_default(),
            })
            .collect()
    }

    fn visible_lines_window(
        &self,
        start: usize,
        height: usize,
        start_col: usize,
        width: usize,
    ) -> Vec<LineView<'_>> {
        self.try_visible_lines_window(start, height, start_col, width)
            .unwrap_or_default()
    }

    fn try_visible_lines_window(
        &self,
        start: usize,
        height: usize,
        start_col: usize,
        width: usize,
    ) -> io::Result<Vec<LineView<'_>>> {
        let end = (start + height).min(self.index.line_count());
        (start..end)
            .map(|row| {
                self.try_window_to_cow(
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
        Some(self.index.total_bytes())
    }

    fn search_text_segment(&self, byte_offset: usize, max_bytes: usize) -> Option<Cow<'_, str>> {
        self.search_text_segment(byte_offset, max_bytes)
    }

    fn set_cursor(&mut self, cursor: Cursor) {
        let row = cursor.row.min(self.line_count().saturating_sub(1));
        let col = cursor.col.min(self.current_line_char_len(row));
        let cursor = Cursor { row, col };
        if self.cursor != cursor {
            self.undo_stack.finish_run();
            self.cursor = cursor;
            self.sync_cursor_byte_offset();
        }
    }

    fn finish_undo_group(&mut self) {
        self.undo_stack.finish_run();
    }

    fn text_range(&self, start: Cursor, end: Cursor) -> io::Result<String> {
        let (start, end) = self.clamped_ordered_range(start, end);
        self.try_slice_to_string(
            self.byte_offset_at(start.row, start.col),
            self.byte_offset_at(end.row, end.col),
        )
    }

    fn replace_range(&mut self, start: Cursor, end: Cursor, text: &str) -> io::Result<bool> {
        self.undo_stack.finish_run();
        let (start, end) = self.clamped_ordered_range(start, end);
        let start_byte = self.byte_offset_at(start.row, start.col);
        let end_byte = self.byte_offset_at(end.row, end.col);
        if start_byte == end_byte && text.is_empty() {
            return Ok(false);
        }

        let before = self.capture_cursor_state();
        self.reset_piece_mutation_metrics();
        self.replace_index_range(start_byte, end_byte, text);
        let (removed, inserted) = self.splice_replacement(start_byte, end_byte, text);
        self.cursor = cursor_after_text(start, text);
        self.cursor_byte_offset = start_byte + text.len();
        self.record_replacement(before, start_byte, removed, inserted);
        self.undo_stack.finish_run();
        Ok(true)
    }

    fn replace_ranges(&mut self, ranges: &[(Cursor, Cursor)], text: &str) -> io::Result<usize> {
        self.undo_stack.finish_run();
        let mut ranges = self.snapshot_ranges(ranges)?;
        ranges.retain(|range| range.start_byte != range.end_byte || !text.is_empty());
        if ranges.is_empty() {
            return Ok(0);
        }
        let before = self.capture_cursor_state();
        self.reset_piece_mutation_metrics();
        #[cfg(test)]
        self.reset_line_index_work();
        let mut edits = Vec::with_capacity(ranges.len().saturating_mul(2));
        // Descending snapshot offsets remain valid as higher ranges change.
        // Each mutation stays local in both the PieceTree and block LineIndex,
        // so the batch needs no document-wide coalesce or index rebuild.
        for range in &ranges {
            self.replace_index_range(range.start_byte, range.end_byte, text);
            let (removed, inserted) =
                self.splice_replacement(range.start_byte, range.end_byte, text);
            if !removed.is_empty() {
                edits.push(PieceEdit::Delete {
                    at: range.start_byte,
                    pieces: removed,
                });
            }
            if !inserted.is_empty() {
                edits.push(PieceEdit::Insert {
                    at: range.start_byte,
                    pieces: inserted,
                });
            }
        }
        let cursor_range = ranges.last().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "replacement batch unexpectedly has no ranges",
            )
        })?;
        self.cursor = cursor_after_text(cursor_range.start, text);
        self.cursor_byte_offset = cursor_range.start_byte + text.len();
        if self.recording {
            self.record_transaction(Transaction {
                before,
                after: self.capture_cursor_state(),
                edits,
                id: 0,
            });
        }
        self.undo_stack.finish_run();
        Ok(ranges.len())
    }

    fn to_string(&self) -> String {
        self.slice_to_string(0, self.index.total_bytes())
    }

    fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        self.pieces.try_for_each(|piece| {
            let range = piece.start..piece.start + piece.len;
            match piece.source {
                Source::Original => self.original.write_slice(range, out)?,
                Source::Add => out.write_all(self.add[range].as_bytes())?,
            }
            Ok(())
        })
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
        if ch == '\n' {
            self.insert_newline();
            return;
        }
        let before = self.capture_cursor_state();
        let at = self.cursor_byte_offset;
        let inserted = self.insert_at_cursor(ch);
        self.index.insert_bytes(at, ch.len_utf8());
        if self.recording {
            self.record_typing_history(before, self.capture_cursor_state(), at, inserted);
        }
    }

    fn insert_newline(&mut self) {
        self.undo_stack.finish_run();
        let before = self.capture_cursor_state();
        let at = self.cursor_byte_offset;
        let inserted = self.insert_at_cursor('\n');
        self.index.insert_newline(at);
        if self.recording {
            let after = self.capture_cursor_state();
            self.record_transaction(Transaction {
                before,
                after,
                edits: vec![PieceEdit::Insert {
                    at,
                    pieces: vec![inserted],
                }],
                id: 0,
            });
        }
        self.undo_stack.finish_run();
    }

    fn delete_back(&mut self) {
        if self.cursor.col > 0 {
            self.delete_previous_char();
        } else if self.cursor.row > 0 {
            self.join_with_previous_line();
        } else {
            self.undo_stack.finish_run();
        }
    }

    fn delete_forward(&mut self) {
        let len = self.current_line_char_len(self.cursor.row);
        if self.cursor.col < len {
            self.delete_next_char();
        } else if self.cursor.row + 1 < self.line_count() {
            self.join_with_next_line();
        } else {
            self.undo_stack.finish_run();
        }
    }

    fn move_left(&mut self) {
        self.undo_stack.finish_run();
        self.move_left_internal();
        self.sync_cursor_byte_offset();
    }

    fn move_right(&mut self) {
        self.undo_stack.finish_run();
        self.move_right_internal();
        self.sync_cursor_byte_offset();
    }

    fn move_up(&mut self) {
        self.undo_stack.finish_run();
        self.move_up_internal();
        self.sync_cursor_byte_offset();
    }

    fn move_down(&mut self) {
        self.undo_stack.finish_run();
        self.move_down_internal();
        self.sync_cursor_byte_offset();
    }

    fn undo(&mut self) {
        self.undo_stack.finish_run();
        if let Some(tx) = self.undo_stack.pop_undo() {
            let was_recording = self.recording;
            self.recording = false;
            self.reset_piece_mutation_metrics();
            for edit in tx.edits.iter().rev() {
                self.apply_inverse_edit(edit);
            }
            self.cursor = tx.before.cursor;
            self.cursor_byte_offset = tx.before.byte_offset;
            self.undo_stack.push_redo(tx);
            self.recording = was_recording;
        }
    }

    fn redo(&mut self) {
        self.undo_stack.finish_run();
        if let Some(tx) = self.undo_stack.pop_redo() {
            let was_recording = self.recording;
            self.recording = false;
            self.reset_piece_mutation_metrics();
            for edit in &tx.edits {
                self.apply_edit(edit);
            }
            self.cursor = tx.after.cursor;
            self.cursor_byte_offset = tx.after.byte_offset;
            self.undo_stack.push_undo(tx);
            self.recording = was_recording;
        }
    }

    fn edit_history_position(&self) -> u64 {
        self.undo_stack.current_history_position()
    }

    fn content_revision(&self) -> u64 {
        self.undo_stack.content_revision()
    }

    fn is_history_position_retained(&self, position: u64) -> bool {
        self.history_position_is_retained(position)
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
    /// Validate snapshot coordinates before mutation, reject ambiguous overlap,
    /// and return the ranges from the document end toward its start.
    fn snapshot_ranges(&self, ranges: &[(Cursor, Cursor)]) -> io::Result<Vec<SnapshotRange>> {
        let mut snapshot_ranges = Vec::with_capacity(ranges.len());
        for &(first, second) in ranges {
            let first = self.valid_snapshot_cursor(first)?;
            let second = self.valid_snapshot_cursor(second)?;
            let (start, end) = if (first.row, first.col) <= (second.row, second.col) {
                (first, second)
            } else {
                (second, first)
            };
            snapshot_ranges.push(SnapshotRange {
                start,
                start_byte: self.byte_offset_at(start.row, start.col),
                end_byte: self.byte_offset_at(end.row, end.col),
            });
        }

        snapshot_ranges.sort_by(|left, right| {
            left.start_byte
                .cmp(&right.start_byte)
                .then(left.end_byte.cmp(&right.end_byte))
        });
        for pair in snapshot_ranges.windows(2) {
            let [left, right] = pair else {
                continue;
            };
            if left.end_byte > right.start_byte || left.start_byte == right.start_byte {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "replacement ranges must be non-overlapping snapshot ranges",
                ));
            }
        }
        snapshot_ranges.reverse();
        Ok(snapshot_ranges)
    }

    fn valid_snapshot_cursor(&self, cursor: Cursor) -> io::Result<Cursor> {
        if cursor.row >= self.line_count() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "replacement range row exceeds the snapshot",
            ));
        }
        let line_len = self.current_line_char_len(cursor.row);
        if cursor.col > line_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "replacement range column exceeds the snapshot",
            ));
        }
        Ok(cursor)
    }

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
            char_len: Some(text.chars().count()),
        };
        self.add_scalars.append(text);
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
        self.record_transaction(Transaction {
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
        self.reset_piece_mutation_metrics();
        let removed = self.delete_byte_range(start, end);
        self.cursor.col -= 1;
        self.cursor_byte_offset = start;
        self.index.delete_bytes(start, end - start);
        self.record_delete(UndoRun::Backspace, before, start, removed);
    }

    fn delete_next_char(&mut self) {
        let start = self.byte_offset_at(self.cursor.row, self.cursor.col);
        let end = self.byte_offset_at(self.cursor.row, self.cursor.col + 1);
        let before = self.capture_cursor_state();
        self.reset_piece_mutation_metrics();
        let removed = self.delete_byte_range(start, end);
        self.index.delete_bytes(start, end - start);
        self.record_delete(UndoRun::DeleteForward, before, start, removed);
    }

    fn join_with_previous_line(&mut self) {
        self.undo_stack.finish_run();
        let next_start = self.byte_offset_at(self.cursor.row, 0);
        if next_start == 0 {
            return;
        }
        let previous_len = self.current_line_char_len(self.cursor.row - 1);
        let before = self.capture_cursor_state();
        self.reset_piece_mutation_metrics();
        let removed = self.delete_byte_range(next_start - 1, next_start);
        self.cursor.row -= 1;
        self.cursor.col = previous_len;
        self.sync_cursor_byte_offset();
        let _ = self.index.delete_newline(next_start - 1);
        self.record_independent_delete(before, next_start - 1, removed);
        self.undo_stack.finish_run();
    }

    fn join_with_next_line(&mut self) {
        self.undo_stack.finish_run();
        let next_start = self.byte_offset_at(self.cursor.row + 1, 0);
        if next_start == 0 {
            return;
        }
        let newline = next_start - 1;
        let before = self.capture_cursor_state();
        self.reset_piece_mutation_metrics();
        let removed = self.delete_byte_range(newline, next_start);
        let _ = self.index.delete_newline(newline);
        self.record_independent_delete(before, newline, removed);
        self.undo_stack.finish_run();
    }

    fn record_delete(
        &mut self,
        run: UndoRun,
        before: crate::buffer::undo::CursorState,
        at: usize,
        pieces: Vec<super::types::Piece>,
    ) {
        if self.recording && !pieces.is_empty() {
            self.record_delete_history(run, before, self.capture_cursor_state(), at, pieces);
        }
    }

    fn record_independent_delete(
        &mut self,
        before: crate::buffer::undo::CursorState,
        at: usize,
        pieces: Vec<Piece>,
    ) {
        if self.recording && !pieces.is_empty() {
            self.record_transaction(Transaction {
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
                let _ = self.index.replace_byte_range(*at, *at + len, 0, &[]);
                self.delete_byte_range(*at, *at + len);
            }
            PieceEdit::Delete { at, pieces } => {
                let (len, newlines) = self.piece_sequence_line_metadata(pieces);
                let _ = self.index.replace_byte_range(*at, *at, len, &newlines);
                self.insert_pieces_at(*at, pieces);
            }
        }
    }

    fn apply_edit(&mut self, edit: &PieceEdit) {
        match edit {
            PieceEdit::Insert { at, pieces } => {
                let (len, newlines) = self.piece_sequence_line_metadata(pieces);
                let _ = self.index.replace_byte_range(*at, *at, len, &newlines);
                self.insert_pieces_at(*at, pieces);
            }
            PieceEdit::Delete { at, pieces } => {
                let len = pieces.iter().map(|piece| piece.len).sum::<usize>();
                let _ = self.index.replace_byte_range(*at, *at + len, 0, &[]);
                self.delete_byte_range(*at, *at + len);
            }
        }
    }

    fn replace_index_range(&mut self, start: usize, end: usize, text: &str) {
        let newlines = text
            .match_indices('\n')
            .map(|(byte, _)| byte)
            .collect::<Vec<_>>();
        let _ = self
            .index
            .replace_byte_range(start, end, text.len(), &newlines);
    }
}
