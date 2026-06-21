//! Line index for fast line access and cursor <-> offset mapping (Phase 1B).
//!
//! Purpose: own the line start byte offsets over the logical document for O(1-ish)
//!          line queries and efficient cursor mapping. Rebuild bridge in 1B-a,
//!          incremental updates targeted in 1B-b.
//! Owns: line_starts vec + total_bytes.
//! Must not: depend on PieceTable internals (Piece/Source live in piece_table).
//!           UI, LLM, or Project code.
//! Invariants:
//! - line_starts[0] == 0 (or doc empty)
//! - line_starts are strictly increasing, last may equal total_bytes (for trailing nl or end)
//! - total_bytes is the logical byte length
//! - Rebuild or incremental update keeps this consistent with pieces after every edit.
//! Phase: 1B

/// Line index over global logical byte offsets (one per line start).
/// Public(crate) for use by PieceTable; the public module exists to avoid
/// duplicate LineIndex concepts.
#[derive(Clone, Debug, Default)]
pub(crate) struct LineIndex {
    /// Global logical byte offsets of the start of each line.
    pub(crate) line_starts: Vec<usize>,
    pub(crate) total_bytes: usize,
}

impl LineIndex {
    pub(crate) fn new() -> Self {
        Self {
            line_starts: vec![0],
            total_bytes: 0,
        }
    }

    pub(crate) fn line_count(&self) -> usize {
        self.line_starts.len().max(1)
    }

    pub(crate) fn line_start_byte(&self, row: usize) -> usize {
        let n = self.line_starts.len();
        if n == 0 {
            return 0;
        }
        self.line_starts[row.min(n - 1)]
    }

    /// Byte offset of the end of the line content (not including the terminating '\n').
    pub(crate) fn line_end_byte(&self, row: usize) -> usize {
        let n = self.line_starts.len();
        if n == 0 {
            return 0;
        }
        let r = row.min(n - 1);
        if r + 1 < n {
            self.line_starts[r + 1].saturating_sub(1)
        } else {
            self.total_bytes
        }
    }

    pub(crate) fn row_for_byte(&self, byte: usize) -> usize {
        let n = self.line_starts.len();
        if n == 0 {
            return 0;
        }
        let mut row = 0;
        for i in 1..n {
            if byte < self.line_starts[i] {
                break;
            }
            row = i;
        }
        row.min(n - 1)
    }

    // NOTE: rebuild_from_pieces lives with PieceTable (it needs internal Piece + Source).
    // PT calls it to produce a LineIndex and assigns it.
}
