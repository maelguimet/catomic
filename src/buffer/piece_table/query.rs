//! Query methods and helpers for PieceTable (Phases 1B-1C).
//!
//! Includes slice_to_string (avoids full materialization), logical helpers,
//! split_point, and within-line measurements that will use the LineIndex.

use super::types::{PieceTable, Source};
use std::io;

impl PieceTable {
    /// Return the logical text for the byte range [start, end).
    /// Uses piece_starts for bounded lookup of start piece (no full head scan).
    pub(crate) fn slice_to_string(&self, start: usize, end: usize) -> String {
        self.try_slice_to_string(start, end).unwrap_or_default()
    }

    pub(crate) fn try_slice_to_string(&self, start: usize, end: usize) -> io::Result<String> {
        if start >= end || self.pieces.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::new();
        let i = self.find_piece_for_byte(start);
        let mut acc = self.piece_starts.get(i).copied().unwrap_or(0);
        for p in &self.pieces[i..] {
            let p_end = acc + p.len;

            if acc >= end {
                break;
            }

            if p_end <= start {
                acc = p_end;
                continue;
            }

            // overlap
            let local_start = if acc < start { start - acc } else { 0 };
            let local_end = if p_end > end { end - acc } else { p.len };
            if local_end > local_start {
                let source_range = p.start + local_start..p.start + local_end;
                match p.source {
                    Source::Original => self.original.try_push_slice(source_range, &mut out)?,
                    Source::Add => out.push_str(&self.add[source_range]),
                }
            }
            acc = p_end;
        }
        Ok(out)
    }

    /// Binary search on piece_starts for the piece containing or nearest before off.
    /// Returns index clamped to last piece.
    fn find_piece_for_byte(&self, off: usize) -> usize {
        let ps = &self.piece_starts;
        let np = ps.len();
        if np == 0 {
            return 0;
        }
        let mut lo = 0usize;
        let mut hi = np;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if ps[mid] <= off {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo.saturating_sub(1).min(np - 1)
    }

    /// Find (piece_index, local_byte_offset) for a global logical byte offset.
    pub(crate) fn split_point(&self, off: usize) -> (usize, usize) {
        if self.pieces.is_empty() {
            return (0, 0);
        }
        let i = self.find_piece_for_byte(off);
        let pstart = self.piece_starts[i];
        let plen = self.pieces[i].len;
        let local = off.saturating_sub(pstart).min(plen);
        (i, local)
    }

    /// Char length of a logical line (uses index when possible + slice).
    pub(crate) fn current_line_char_len(&self, row: usize) -> usize {
        let n = self.index.line_starts.len();
        if n == 0 {
            return 0;
        }
        let row = row.min(n.saturating_sub(1));
        let start = self.index.line_starts[row];
        let end = if row + 1 < n {
            self.index.line_starts[row + 1].saturating_sub(1)
        } else {
            self.index.total_bytes
        };
        if start >= end {
            return 0;
        }
        self.slice_to_string(start, end).chars().count()
    }

    /// Byte offset from (row, char-col) using the line index + local scan.
    /// Much cheaper than full logical_text for large docs.
    pub(crate) fn byte_offset_at(&self, mut row: usize, mut col: usize) -> usize {
        let n = self.index.line_starts.len();
        if n == 0 || self.index.total_bytes == 0 {
            return 0;
        }
        row = row.min(n.saturating_sub(1));
        let line_start = self.index.line_starts[row];
        let line_end = if row + 1 < n {
            self.index.line_starts[row + 1].saturating_sub(1)
        } else {
            self.index.total_bytes
        };
        let line_str = self.slice_to_string(line_start, line_end);
        let n_chars = line_str.chars().count();
        col = col.min(n_chars);

        let mut b = 0usize;
        let mut seen = 0usize;
        for ch in line_str.chars() {
            if seen == col {
                break;
            }
            b += ch.len_utf8();
            seen += 1;
        }
        line_start + b
    }
}
