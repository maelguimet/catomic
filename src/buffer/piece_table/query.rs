//! Purpose: query logical PieceTable ranges without full-buffer materialization.
//! Owns: piece overlap traversal, scalar counts, cursor byte mapping,
//!   compatibility string slices, and piece lookup/split points.
//! Must not: mutate pieces, perform App/render policy, or know repository/network work.
//! Invariants: source ranges respect UTF-8 boundaries and logical offsets are global.

use super::types::{PieceTable, Source};
use std::io;

impl PieceTable {
    /// Borrow a bounded logical source range for streaming search when possible.
    pub(crate) fn search_text_segment(
        &self,
        byte_offset: usize,
        max_bytes: usize,
    ) -> Option<std::borrow::Cow<'_, str>> {
        if byte_offset >= self.index.total_bytes() || max_bytes == 0 {
            return None;
        }
        let (index, local) = self.split_point(byte_offset);
        let piece = self.pieces.get(index)?;
        let source_start = piece.start + local;
        match piece.source {
            Source::Original => self
                .original
                .search_text_segment(source_start..piece.start + piece.len, max_bytes)
                .ok(),
            Source::Add => {
                let mut source_end = source_start + (piece.len - local).min(max_bytes);
                while source_end < piece.start + piece.len && !self.add.is_char_boundary(source_end)
                {
                    source_end += 1;
                }
                Some(std::borrow::Cow::Borrowed(
                    &self.add[source_start..source_end],
                ))
            }
        }
    }
    /// Return the logical text for the byte range [start, end).
    /// Uses subtree byte summaries for bounded lookup of the start piece.
    pub(crate) fn slice_to_string(&self, start: usize, end: usize) -> String {
        self.try_slice_to_string(start, end).unwrap_or_default()
    }

    pub(crate) fn try_slice_to_string(&self, start: usize, end: usize) -> io::Result<String> {
        if start >= end || self.pieces.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::new();
        let i = self.find_piece_for_byte(start);
        let mut acc = self.pieces.logical_start(i);
        self.pieces.try_for_each_from(i, |p| -> io::Result<bool> {
            let p_end = acc + p.len;

            if acc >= end {
                return Ok(false);
            }

            if p_end <= start {
                acc = p_end;
                return Ok(true);
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
            Ok(true)
        })?;
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
        let mut piece_start = self.pieces.logical_start(first);
        self.pieces
            .try_for_each_from(first, |piece| -> io::Result<bool> {
                let piece_end = piece_start + piece.len;
                if piece_start >= end {
                    return Ok(false);
                }
                let local_start = start.saturating_sub(piece_start).min(piece.len);
                let local_end = end.saturating_sub(piece_start).min(piece.len);
                if local_start < local_end {
                    let source_range = piece.start + local_start..piece.start + local_end;
                    if !visit(piece.source, source_range, piece_start + local_start)? {
                        return Ok(false);
                    }
                }
                piece_start = piece_end;
                Ok(true)
            })?;
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

    /// Bounded lookup through subtree byte summaries.
    fn find_piece_for_byte(&self, off: usize) -> usize {
        self.pieces.locate(off).0
    }

    /// Find (piece_index, local_byte_offset) for a global logical byte offset.
    pub(crate) fn split_point(&self, off: usize) -> (usize, usize) {
        if self.pieces.is_empty() {
            return (0, 0);
        }
        self.pieces.locate(off)
    }

    /// Char length of a logical line using per-source scalar metadata.
    pub(crate) fn current_line_char_len(&self, row: usize) -> usize {
        let row = row.min(self.index.line_count().saturating_sub(1));
        let start = self.index.line_start_byte(row);
        let end = self.index.line_end_byte(row);
        self.try_char_count(start, end).unwrap_or(0)
    }

    /// Byte offset from (row, char-col) using the line index + local scan.
    /// Much cheaper than full logical_text for large docs.
    pub(crate) fn byte_offset_at(&self, mut row: usize, mut col: usize) -> usize {
        let n = self.index.line_count();
        if self.index.total_bytes() == 0 {
            return 0;
        }
        row = row.min(n.saturating_sub(1));
        let line_start = self.index.line_start_byte(row);
        let line_end = self.index.line_end_byte(row);
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
