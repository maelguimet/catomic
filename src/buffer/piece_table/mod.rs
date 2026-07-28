//! Purpose: maintain PieceTable index metadata, piece-prefix, and cursor-state helpers.
//! Owns: inserted-piece line metadata, piece coalescing, and history cursor capture.
//! Must not: external-service/config policy or UI expansion.
//! Invariants:
//! - Pieces UTF-8 char-boundary safe, cover logical doc.
//! - index remains consistent through local edit deltas.
//! - cursor_byte_offset always matches (row, col) position.
//! - Buffer adaptation and mutation orchestration live in focused submodules.
//!

mod buffer_impl;
mod construct;
mod edit;
mod file_original;
mod query;
pub(crate) mod types;

use crate::buffer::undo::CursorState;
pub use types::PieceTable;
use types::{Piece, Source};

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
    pub(crate) line_index_blocks_touched: usize,
    pub(crate) line_index_summary_nodes_updated: usize,
    pub(crate) descriptor_read_bytes: usize,
    pub(crate) descriptor_metadata_checks: usize,
}

impl PieceTable {
    pub(crate) fn has_edit_history(&self) -> bool {
        self.undo_stack.has_history()
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
        let line_index_work = self.index.work();
        let retained_bytes = self.original.retained_bytes()
            + self.add.capacity()
            + self.pieces.capacity() * std::mem::size_of::<Piece>()
            + self.index.retained_bytes()
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
            // The block-local representation neither scans document bytes nor
            // shifts a tail of absolute line starts during ordinary edits.
            line_index_scanned_bytes: 0,
            line_index_shifted_entries: 0,
            line_index_blocks_touched: line_index_work.blocks_touched,
            line_index_summary_nodes_updated: line_index_work.summary_nodes_updated,
            descriptor_read_bytes: self.original.file_read_bytes(),
            descriptor_metadata_checks: self.original.metadata_check_count(),
        }
    }

    fn piece_sequence_line_metadata(&self, pieces: &[Piece]) -> (usize, Vec<usize>) {
        let mut byte_len = 0usize;
        let mut newline_offsets = Vec::new();
        for piece in pieces {
            let range = piece.start..piece.start + piece.len;
            match piece.source {
                Source::Original => {
                    self.original.for_each_newline(range, |source_byte| {
                        newline_offsets.push(byte_len + source_byte - piece.start);
                    });
                }
                Source::Add => {
                    for (local_byte, _) in self.add[range].match_indices('\n') {
                        newline_offsets.push(byte_len + local_byte);
                    }
                }
            }
            byte_len += piece.len;
        }
        (byte_len, newline_offsets)
    }

    #[cfg(test)]
    pub(crate) fn reset_line_index_work(&mut self) {
        self.index.reset_work();
    }

    #[cfg(test)]
    pub(crate) fn line_index_work(&self) -> crate::buffer::line_index::LineIndexWork {
        self.index.work()
    }
}
