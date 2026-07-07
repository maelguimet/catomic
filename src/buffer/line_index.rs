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
    pub(crate) fn from_text(text: &str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, _) in text.match_indices('\n') {
            line_starts.push(i + 1);
        }
        Self {
            line_starts,
            total_bytes: text.len(),
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
        let starts = &self.line_starts;
        let n = starts.len();
        if n == 0 {
            return 0;
        }
        // Binary search for the rightmost start <= byte.
        let mut lo = 0usize;
        let mut hi = n;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if starts[mid] <= byte {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo.saturating_sub(1).min(n - 1)
    }

    // NOTE: rebuild_from_pieces lives with PieceTable (it needs internal Piece + Source).
    // PT calls it to produce a LineIndex and assigns it.
}

#[cfg(test)]
mod tests {
    use super::LineIndex;

    #[test]
    fn from_text_empty_has_single_start() {
        let index = LineIndex::from_text("");

        assert_eq!(index.line_starts, vec![0]);
        assert_eq!(index.total_bytes, 0);
        assert_eq!(index.line_count(), 1);
    }

    #[test]
    fn from_text_records_lf_line_starts() {
        let index = LineIndex::from_text("one\ntwo\n");

        assert_eq!(index.line_starts, vec![0, 4, 8]);
        assert_eq!(index.total_bytes, 8);
        assert_eq!(index.line_count(), 3);
    }

    #[test]
    fn from_text_uses_byte_offsets_for_multibyte_content() {
        let index = LineIndex::from_text("é\n猫\nx");

        assert_eq!(index.line_starts, vec![0, 3, 7]);
        assert_eq!(index.total_bytes, 8);
        assert_eq!(index.line_start_byte(2), 7);
    }
}
