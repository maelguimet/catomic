//! Query methods and helpers for PieceTable (Phase 1B).
//!
//! Includes slice_to_string (avoids full materialization), logical helpers,
//! split_point, and within-line measurements that will use the LineIndex.

use super::types::{PieceTable, Source};

impl PieceTable {
    /// Return the logical text for the byte range [start, end).
    /// Walks only the relevant pieces. Core primitive for index-driven queries.
    pub(crate) fn slice_to_string(&self, start: usize, end: usize) -> String {
        if start >= end {
            return String::new();
        }
        let mut out = String::new();
        let mut acc = 0usize;
        for p in &self.pieces {
            let p_end = acc + p.len;
            if p_end <= start || acc >= end {
                acc = p_end;
                continue;
            }
            // overlap
            let local_start = if acc < start { start - acc } else { 0 };
            let local_end = if p_end > end { end - acc } else { p.len };
            if local_end > local_start {
                let src = match p.source {
                    Source::Original => &self.original,
                    Source::Add => &self.add,
                };
                out.push_str(&src[p.start + local_start..p.start + local_end]);
            }
            acc = p_end;
        }
        out
    }

    /// Full logical text. Retained for to_string and as fallback during 1B transition.
    /// Hot query paths (line/visible) should use slice + index instead.
    pub(crate) fn logical_text(&self) -> String {
        let cap: usize = self.pieces.iter().map(|p| p.len).sum();
        let mut out = String::with_capacity(cap);
        for p in &self.pieces {
            let src = match p.source {
                Source::Original => &self.original,
                Source::Add => &self.add,
            };
            out.push_str(&src[p.start..p.start + p.len]);
        }
        out
    }

    pub(crate) fn logical_lines(&self) -> Vec<String> {
        // During transition may still be used by some paths.
        self.logical_text()
            .split('\n')
            .map(|s| s.to_string())
            .collect()
    }

    /// Find (piece_index, local_byte_offset) for a global logical byte offset.
    pub(crate) fn split_point(&self, off: usize) -> (usize, usize) {
        let mut acc = 0usize;
        for (i, p) in self.pieces.iter().enumerate() {
            if off <= acc + p.len {
                return (i, off - acc);
            }
            acc += p.len;
        }
        let last = self.pieces.len() - 1;
        (last, self.pieces[last].len)
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
