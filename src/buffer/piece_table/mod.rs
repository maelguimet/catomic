//! Purpose: maintain PieceTable line-index and cursor-state helpers.
//! Owns: inserted-piece line metadata, mutation metrics, and history cursor capture.
//! Must not: external-service/config policy or UI expansion.
//! Invariants:
//! - Pieces UTF-8 char-boundary safe, cover logical doc.
//! - line and piece indexes remain consistent after every edit.
//! - cursor_byte_offset always matches (row, col) position.
//! - Buffer adaptation and mutation orchestration live in focused submodules.
//!

mod buffer_impl;
mod construct;
mod edit;
pub(crate) mod file_original;
mod piece_tree;
mod query;
mod retention;
mod scalar_index;
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

    pub(crate) fn undo_transaction_count(&self) -> usize {
        self.undo_stack.undo_transaction_count()
    }

    pub(crate) fn needs_page_retention(&self) -> bool {
        self.has_edit_history() || self.undo_stack.current_history_position() != 0
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
    pub(crate) fn last_piece_mutation(&self) -> types::PieceMutationMetrics {
        self.last_piece_mutation
    }

    #[cfg(test)]
    pub(crate) fn undo_history_metrics(&self) -> (usize, usize, usize) {
        (
            self.undo_stack.undo_transaction_count(),
            self.undo_stack.history_container_count(),
            self.undo_stack.retained_history_bytes(),
        )
    }

    #[cfg(test)]
    pub(crate) fn perf_stats(&self) -> PieceTablePerfStats {
        let (history_transactions, history_bytes) = self.undo_stack.perf_stats();
        let line_index_work = self.index.work();
        let retained_bytes = self.original.retained_bytes()
            + self.add.capacity()
            + self.add_scalars.retained_bytes()
            + self.pieces.retained_bytes()
            + self.index.retained_bytes()
            + history_bytes;
        PieceTablePerfStats {
            pieces: self.pieces.len(),
            document_lines: self.index.line_count(),
            add_buffer_bytes: self.add.len(),
            history_transactions,
            history_bytes,
            retained_bytes,
            retained_metadata_bytes: self
                .original
                .retained_metadata_bytes()
                .saturating_add(self.index.retained_bytes()),
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
