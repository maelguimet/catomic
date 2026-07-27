//! Purpose: maintain PieceTable index, piece-prefix, and cursor-state helpers.
//! Owns: index rebuild/incremental updates, piece coalescing, and history cursor capture.
//! Must not: external-service/config policy or UI expansion.
//! Invariants:
//! - Pieces UTF-8 char-boundary safe, cover logical doc.
//! - index consistent after edit (rebuild bridge first; incremental later).
//! - cursor_byte_offset always matches (row, col) position.
//! - Buffer adaptation and mutation orchestration live in focused submodules.
//!

mod buffer_impl;
mod construct;
mod edit;
mod file_original;
mod query;
pub(crate) mod types;

use crate::buffer::line_index::LineIndex;

use crate::buffer::undo::CursorState;
pub use types::PieceTable;
use types::{OriginalBacking, Piece, Source};

#[cfg(test)]
pub(crate) struct PieceTablePerfStats {
    pub(crate) pieces: usize,
    pub(crate) document_lines: usize,
    pub(crate) add_buffer_bytes: usize,
    pub(crate) history_transactions: usize,
    pub(crate) history_bytes: usize,
    pub(crate) retained_bytes: usize,
    pub(crate) retained_metadata_bytes: usize,
    pub(crate) line_index_scanned_bytes: usize,
    pub(crate) line_index_shifted_entries: usize,
    pub(crate) descriptor_read_bytes: usize,
    pub(crate) descriptor_metadata_checks: usize,
}

impl PieceTable {
    pub(crate) fn has_edit_history(&self) -> bool {
        self.undo_stack.has_history()
    }

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
        #[cfg(test)]
        {
            self.line_index_scanned_bytes +=
                self.pieces.iter().map(|piece| piece.len).sum::<usize>();
        }
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

    #[cfg(test)]
    pub(crate) fn perf_stats(&self) -> PieceTablePerfStats {
        let (history_transactions, history_bytes) = self.undo_stack.perf_stats();
        let retained_bytes = self.original.retained_bytes()
            + self.add.capacity()
            + self.pieces.capacity() * std::mem::size_of::<Piece>()
            + self.index.line_starts.capacity() * std::mem::size_of::<usize>()
            + self.piece_starts.capacity() * std::mem::size_of::<usize>()
            + history_bytes;
        PieceTablePerfStats {
            pieces: self.pieces.len(),
            document_lines: self.index.line_count(),
            add_buffer_bytes: self.add.len(),
            history_transactions,
            history_bytes,
            retained_bytes,
            retained_metadata_bytes: self.original.retained_metadata_bytes(),
            line_index_scanned_bytes: self.line_index_scanned_bytes,
            line_index_shifted_entries: self.line_index_shifted_entries,
            descriptor_read_bytes: self.original.file_read_bytes(),
            descriptor_metadata_checks: self.original.metadata_check_count(),
        }
    }

    /// Incremental update for index when edit does not add/remove a '\n'.
    /// Shifts subsequent line starts and total_bytes.
    fn adjust_index_for_simple_delta(&mut self, at_byte: usize, delta: isize) {
        if delta == 0 {
            return;
        }
        let row = self.index.row_for_byte(at_byte);
        #[cfg(test)]
        {
            self.line_index_shifted_entries +=
                self.index.line_starts.len().saturating_sub(row + 1);
        }
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
        #[cfg(test)]
        {
            self.line_index_shifted_entries +=
                self.index.line_starts.len().saturating_sub(row + 1);
        }
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
        #[cfg(test)]
        {
            self.line_index_shifted_entries += self.index.line_starts.len().saturating_sub(idx);
        }
        // Shift the boundary and tail down by 1 for removed byte.
        for ls in &mut self.index.line_starts[idx..] {
            *ls = ls.saturating_sub(1);
        }
        self.index.line_starts.remove(idx);
        self.index.total_bytes = self.index.total_bytes.saturating_sub(1);
    }
}
