//! LineIndex implementation (Phase 1B).
//!
//! Rebuild-from-pieces first for a solid correctness bridge.
//! Incremental maintenance comes later after tests are strong.

use super::types::{LineIndex, Piece, Source};

impl LineIndex {
    /// Rebuild the line start index by walking pieces without full materialization.
    pub fn rebuild_from_pieces(original: &str, add: &str, pieces: &[Piece]) -> Self {
        let mut line_starts = vec![0usize];
        let mut acc: usize = 0;

        for p in pieces {
            let src = match p.source {
                Source::Original => original,
                Source::Add => add,
            };
            let pbytes = &src.as_bytes()[p.start..p.start + p.len];
            for (i, &b) in pbytes.iter().enumerate() {
                if b == b'\n' {
                    line_starts.push(acc + i + 1);
                }
            }
            acc += p.len;
        }

        // For the completely empty case we still want [0]
        if line_starts.is_empty() {
            line_starts.push(0);
        }

        LineIndex {
            line_starts,
            total_bytes: acc,
        }
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len().max(1)
    }

    pub fn line_start_byte(&self, row: usize) -> usize {
        let n = self.line_starts.len();
        if n == 0 {
            return 0;
        }
        self.line_starts[row.min(n - 1)]
    }

    /// Byte offset of the end of the line content (not including the terminating \n).
    pub fn line_end_byte(&self, row: usize) -> usize {
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

    pub fn row_for_byte(&self, byte: usize) -> usize {
        // Binary search would be nice; linear is fine for 1B bridge + small docs.
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
}
