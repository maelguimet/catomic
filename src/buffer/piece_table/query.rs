//! Purpose: query logical PieceTable ranges without full-buffer materialization.
//! Owns: piece overlap traversal, scalar counts/windows, cursor byte mapping,
//!   compatibility string slices, and piece lookup/split points.
//! Must not: mutate pieces, perform App/render policy, or know Project/LLM work.
//! Invariants: source ranges respect UTF-8 boundaries; file-backed scalar
//!   windows use bounded checkpoint-assisted reads; logical offsets are global.

use super::types::{PieceTable, Source};
use crate::buffer::CursorContext;
use std::io;

impl PieceTable {
    pub(crate) fn try_cursor_context(
        &self,
        max_before: usize,
        max_after: usize,
    ) -> io::Result<CursorContext> {
        let cursor = self.cursor_byte_offset.min(self.index.total_bytes);
        let start = self.bounded_start(cursor, max_before)?;
        let end = self.bounded_end(cursor, max_after)?;
        let before = self.try_slice_to_string(start, cursor)?;
        let after = self.try_slice_to_string(cursor, end)?;
        Ok(CursorContext {
            before: suffix_chars(&before, max_before),
            after: after.chars().take(max_after).collect(),
        })
    }

    fn bounded_start(&self, cursor: usize, max_chars: usize) -> io::Result<usize> {
        let mut start = cursor.saturating_sub(max_chars.saturating_mul(4));
        while start < cursor && !self.logical_char_boundary(start)? {
            start += 1;
        }
        Ok(start)
    }

    fn bounded_end(&self, cursor: usize, max_chars: usize) -> io::Result<usize> {
        let mut end = cursor
            .saturating_add(max_chars.saturating_mul(4))
            .min(self.index.total_bytes);
        while end > cursor && !self.logical_char_boundary(end)? {
            end -= 1;
        }
        Ok(end)
    }

    fn logical_char_boundary(&self, offset: usize) -> io::Result<bool> {
        if offset == self.index.total_bytes {
            return Ok(true);
        }
        let piece_index = self.find_piece_for_byte(offset);
        let piece = &self.pieces[piece_index];
        let local = offset.saturating_sub(self.piece_starts[piece_index]);
        let source_offset = piece.start.saturating_add(local);
        match piece.source {
            Source::Original => self
                .original
                .owned_is_char_boundary(source_offset)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::Unsupported,
                        "autocomplete context cannot read a descriptor-backed buffer",
                    )
                }),
            Source::Add => Ok(self.add.is_char_boundary(source_offset)),
        }
    }

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
            let local_start = start.saturating_sub(acc);
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

    pub(crate) fn try_char_count(&self, start: usize, end: usize) -> io::Result<usize> {
        if start >= end || self.pieces.is_empty() {
            return Ok(0);
        }
        let mut count = 0usize;
        self.for_each_piece_overlap(start, end, |source, range, _logical_start| {
            count += self.source_char_count(source, range)?;
            Ok(true)
        })?;
        Ok(count)
    }

    pub(crate) fn try_byte_offset_after_chars(
        &self,
        start: usize,
        end: usize,
        mut chars: usize,
    ) -> io::Result<usize> {
        let mut result = end;
        self.for_each_piece_overlap(start, end, |source, range, logical_start| {
            let count = self.source_char_count(source, range.clone())?;
            if chars <= count {
                let source_offset =
                    self.source_byte_offset_at_char(source, range.clone(), chars)?;
                result = logical_start + (source_offset - range.start);
                return Ok(false);
            }
            chars -= count;
            Ok(true)
        })?;
        Ok(result)
    }

    pub(crate) fn try_window_to_string(
        &self,
        start: usize,
        end: usize,
        mut skip: usize,
        width: usize,
    ) -> io::Result<String> {
        if width == 0 || start >= end {
            return Ok(String::new());
        }
        let mut out = String::new();
        let mut remaining = width;
        self.for_each_piece_overlap(start, end, |source, range, _logical_start| {
            let count = self.source_char_count(source, range.clone())?;
            if skip >= count {
                skip -= count;
                return Ok(true);
            }
            let taken = self.source_push_char_window(source, range, skip, remaining, &mut out)?;
            skip = 0;
            remaining -= taken;
            Ok(remaining > 0)
        })?;
        Ok(out)
    }

    fn for_each_piece_overlap(
        &self,
        start: usize,
        end: usize,
        mut visit: impl FnMut(Source, std::ops::Range<usize>, usize) -> io::Result<bool>,
    ) -> io::Result<()> {
        if start >= end || self.pieces.is_empty() {
            return Ok(());
        }
        let first = self.find_piece_for_byte(start);
        let mut piece_start = self.piece_starts.get(first).copied().unwrap_or(0);
        for piece in &self.pieces[first..] {
            let piece_end = piece_start + piece.len;
            if piece_start >= end {
                break;
            }
            let local_start = start.saturating_sub(piece_start).min(piece.len);
            let local_end = end.saturating_sub(piece_start).min(piece.len);
            if local_start < local_end {
                let source_range = piece.start + local_start..piece.start + local_end;
                if !visit(piece.source, source_range, piece_start + local_start)? {
                    break;
                }
            }
            piece_start = piece_end;
        }
        Ok(())
    }

    fn source_char_count(
        &self,
        source: Source,
        range: std::ops::Range<usize>,
    ) -> io::Result<usize> {
        match source {
            Source::Original => self.original.try_char_count(range),
            Source::Add => Ok(self.add[range].chars().count()),
        }
    }

    fn source_byte_offset_at_char(
        &self,
        source: Source,
        range: std::ops::Range<usize>,
        col: usize,
    ) -> io::Result<usize> {
        match source {
            Source::Original => self.original.try_byte_offset_at_char(range, col),
            Source::Add => Ok(range.start
                + self.add[range.clone()]
                    .char_indices()
                    .nth(col)
                    .map_or(range.len(), |(offset, _)| offset)),
        }
    }

    fn source_push_char_window(
        &self,
        source: Source,
        range: std::ops::Range<usize>,
        skip: usize,
        take: usize,
        out: &mut String,
    ) -> io::Result<usize> {
        match source {
            Source::Original => self.original.try_push_char_window(range, skip, take, out),
            Source::Add => {
                let window: String = self.add[range].chars().skip(skip).take(take).collect();
                let taken = window.chars().count();
                out.push_str(&window);
                Ok(taken)
            }
        }
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

    /// Char length of a logical line using per-source scalar metadata.
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
        self.try_char_count(start, end).unwrap_or(0)
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
        let n_chars = self.try_char_count(line_start, line_end).unwrap_or(0);
        col = col.min(n_chars);
        self.try_byte_offset_after_chars(line_start, line_end, col)
            .unwrap_or(line_start)
    }

    #[cfg(test)]
    pub(crate) fn file_original_read_bytes(&self) -> usize {
        self.original.file_read_bytes()
    }
}

fn suffix_chars(text: &str, limit: usize) -> String {
    let mut chars: Vec<char> = text.chars().rev().take(limit).collect();
    chars.reverse();
    chars.into_iter().collect()
}
