//! Purpose: query logical PieceTable ranges without full-buffer materialization.
//! Owns: piece overlap traversal, scalar counts, cursor byte mapping,
//!   compatibility string slices, and piece lookup/split points.
//! Must not: mutate pieces, perform App/render policy, or know repository/network work.
//! Invariants: source ranges respect UTF-8 boundaries and logical offsets are global.

use super::types::{OriginalReadOperation, PieceTable, Source};
use std::borrow::Cow;
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
        self.original.with_read_operation(|original| {
            self.try_slice_to_string_in_read_operation(start, end, original)
        })
    }

    fn try_slice_to_string_in_read_operation(
        &self,
        start: usize,
        end: usize,
        original: &OriginalReadOperation<'_>,
    ) -> io::Result<String> {
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
                    Source::Original => original.try_push_slice(source_range, &mut out)?,
                    Source::Add => out.push_str(&self.add[source_range]),
                }
            }
            acc = p_end;
            Ok(true)
        })?;
        Ok(out)
    }

    /// Return a logical range without copying when it lies in one contiguous
    /// in-memory source piece. File originals deliberately fall back to their
    /// descriptor-aware reader so CRLF normalization remains exact.
    pub(crate) fn try_slice_to_cow(&self, start: usize, end: usize) -> io::Result<Cow<'_, str>> {
        self.original.with_read_operation(|original| {
            self.try_slice_to_cow_in_read_operation(start, end, original)
        })
    }

    fn try_slice_to_cow_in_read_operation<'a>(
        &'a self,
        start: usize,
        end: usize,
        original: &OriginalReadOperation<'_>,
    ) -> io::Result<Cow<'a, str>> {
        let end = end.min(self.pieces.byte_len());
        if start >= end || self.pieces.is_empty() {
            return Ok(Cow::Borrowed(""));
        }
        let (piece_index, local_start) = self.pieces.locate(start);
        let piece = self.pieces.get(piece_index).copied();
        if let Some(piece) = piece {
            let logical_start = self.pieces.logical_start(piece_index);
            let local_end = end.saturating_sub(logical_start);
            if local_end <= piece.len {
                let source_range = piece.start + local_start..piece.start.saturating_add(local_end);
                let borrowed = match piece.source {
                    Source::Original => self.original.borrowed_slice(source_range),
                    Source::Add => Some(&self.add[source_range]),
                };
                if let Some(text) = borrowed {
                    return Ok(Cow::Borrowed(text));
                }
            }
        }
        self.try_slice_to_string_in_read_operation(start, end, original)
            .map(Cow::Owned)
    }

    pub(crate) fn try_char_count(&self, start: usize, end: usize) -> io::Result<usize> {
        self.original.with_read_operation(|original| {
            self.try_char_count_in_read_operation(start, end, original)
        })
    }

    fn try_char_count_in_read_operation(
        &self,
        start: usize,
        end: usize,
        original: &OriginalReadOperation<'_>,
    ) -> io::Result<usize> {
        if start >= end || self.pieces.is_empty() {
            return Ok(0);
        }
        self.pieces
            .try_char_count_in(start..end, |piece, local_range| {
                self.source_char_count_in_read_operation(
                    piece.source,
                    piece.start + local_range.start..piece.start + local_range.end,
                    original,
                )
            })
    }

    pub(crate) fn try_byte_offset_after_chars(
        &self,
        start: usize,
        end: usize,
        chars: usize,
    ) -> io::Result<usize> {
        self.original.with_read_operation(|original| {
            self.try_byte_offset_after_chars_in_read_operation(start, end, chars, original)
        })
    }

    fn try_byte_offset_after_chars_in_read_operation(
        &self,
        start: usize,
        end: usize,
        chars: usize,
        original: &OriginalReadOperation<'_>,
    ) -> io::Result<usize> {
        self.pieces.try_byte_offset_after_chars(
            start..end,
            chars,
            |piece, local_range| {
                self.source_char_count_in_read_operation(
                    piece.source,
                    piece.start + local_range.start..piece.start + local_range.end,
                    original,
                )
            },
            |piece, local_range, scalar| {
                let source_start = piece.start + local_range.start;
                let source_end = piece.start + local_range.end;
                self.source_byte_offset_at_char_in_read_operation(
                    piece.source,
                    source_start..source_end,
                    scalar,
                    original,
                )
                .map(|source_byte| source_byte - piece.start)
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn try_window_to_string(
        &self,
        start: usize,
        end: usize,
        skip: usize,
        width: usize,
    ) -> io::Result<String> {
        self.try_window_to_cow(start, end, skip, width)
            .map(Cow::into_owned)
    }

    #[cfg(test)]
    pub(crate) fn try_window_to_cow(
        &self,
        start: usize,
        end: usize,
        skip: usize,
        width: usize,
    ) -> io::Result<Cow<'_, str>> {
        self.original.with_read_operation(|original| {
            self.try_window_to_cow_in_read_operation(start, end, skip, width, original)
        })
    }

    fn try_window_to_cow_in_read_operation<'a>(
        &'a self,
        start: usize,
        end: usize,
        skip: usize,
        width: usize,
        original: &OriginalReadOperation<'_>,
    ) -> io::Result<Cow<'a, str>> {
        if width == 0 || start >= end {
            return Ok(Cow::Borrowed(""));
        }
        let window_start =
            self.try_byte_offset_after_chars_in_read_operation(start, end, skip, original)?;
        if window_start >= end {
            return Ok(Cow::Borrowed(""));
        }
        let window_end =
            self.try_byte_offset_after_chars_in_read_operation(window_start, end, width, original)?;
        self.try_slice_to_cow_in_read_operation(window_start, window_end, original)
    }

    fn source_char_count_in_read_operation(
        &self,
        source: Source,
        range: std::ops::Range<usize>,
        original: &OriginalReadOperation<'_>,
    ) -> io::Result<usize> {
        match source {
            Source::Original => original.try_char_count(range),
            Source::Add => Ok(self.add_scalars.scalar_count(&self.add, range)),
        }
    }

    pub(crate) fn source_char_count(
        &self,
        source: Source,
        range: std::ops::Range<usize>,
    ) -> io::Result<usize> {
        self.original.with_read_operation(|original| {
            self.source_char_count_in_read_operation(source, range, original)
        })
    }

    fn source_byte_offset_at_char_in_read_operation(
        &self,
        source: Source,
        range: std::ops::Range<usize>,
        col: usize,
        original: &OriginalReadOperation<'_>,
    ) -> io::Result<usize> {
        match source {
            Source::Original => original.try_byte_offset_at_char(range, col),
            Source::Add => Ok(self.add_scalars.byte_at_scalar_in(&self.add, range, col)),
        }
    }

    pub(crate) fn try_visible_lines_window_with_char_counts(
        &self,
        start: usize,
        height: usize,
        start_col: usize,
        width: usize,
        hide_trailing_empty_boundary: bool,
    ) -> io::Result<Vec<(Cow<'_, str>, usize)>> {
        self.original.with_read_operation(|original| {
            let mut line_count = self.index.line_count();
            if hide_trailing_empty_boundary
                && line_count > 1
                && self.index.line_start_byte(line_count - 1)
                    == self.index.line_end_byte(line_count - 1)
            {
                line_count -= 1;
            }
            let end = start.saturating_add(height).min(line_count);
            (start..end)
                .map(|row| {
                    let line_start = self.index.line_start_byte(row);
                    let line_end = self.index.line_end_byte(row);
                    let char_count =
                        self.try_char_count_in_read_operation(line_start, line_end, original)?;
                    let content = self.try_window_to_cow_in_read_operation(
                        line_start, line_end, start_col, width, original,
                    )?;
                    Ok((content, char_count))
                })
                .collect()
        })
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

    #[cfg(test)]
    pub(crate) fn file_original_metadata_check_count(&self) -> usize {
        self.original.metadata_check_count()
    }

    #[cfg(test)]
    pub(crate) fn set_file_read_operation_test_hook(
        &self,
        point: super::types::FileReadOperationTestPoint,
        action: impl FnOnce() + Send + 'static,
    ) {
        self.original
            .set_file_read_operation_test_hook(point, action);
    }

    #[cfg(test)]
    pub(crate) fn take_scalar_visited_bytes(&self) -> usize {
        self.original.take_scalar_visited_bytes() + self.add_scalars.take_visited_bytes()
    }

    #[cfg(test)]
    pub(crate) fn take_scalar_piece_visits(&self) -> usize {
        self.pieces.take_coordinate_node_visits()
    }

    #[cfg(test)]
    pub(crate) fn uses_shared_file_line_index(&self) -> bool {
        self.index.uses_shared_file_metadata()
    }

    #[cfg(test)]
    pub(crate) fn retained_metadata_components(
        &self,
    ) -> Option<super::file_original::FileOriginalMetadataBytes> {
        self.original.file_metadata_bytes().map(|mut bytes| {
            bytes.materialized_line_index = self.index.retained_bytes();
            bytes
        })
    }
}
