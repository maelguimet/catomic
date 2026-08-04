//! Purpose: provide compact, read-only storage for generated preview documents.
//! Owns: one rendered text allocation, compact line starts, and preview navigation.
//! Must not: parse Markdown, retain annotations, mutate text, or perform I/O.
//! Invariants: line starts use u32 while possible and promote losslessly on overflow.

use std::borrow::Cow;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{Buffer, Cursor, LineView};

#[derive(Clone, Debug, PartialEq, Eq)]
enum OffsetStorage {
    Compact(Vec<u32>),
    Wide(Vec<usize>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompactLineStarts {
    storage: OffsetStorage,
}

impl Default for CompactLineStarts {
    fn default() -> Self {
        Self::new()
    }
}

impl CompactLineStarts {
    pub(crate) fn new() -> Self {
        Self {
            storage: OffsetStorage::Compact(vec![0]),
        }
    }

    pub(crate) fn push(&mut self, offset: usize) {
        match &mut self.storage {
            OffsetStorage::Compact(offsets) => {
                if let Ok(offset) = u32::try_from(offset) {
                    offsets.push(offset);
                } else {
                    let mut wide = Vec::with_capacity(offsets.len().saturating_add(1));
                    wide.extend(offsets.iter().map(|offset| *offset as usize));
                    wide.push(offset);
                    self.storage = OffsetStorage::Wide(wide);
                }
            }
            OffsetStorage::Wide(offsets) => offsets.push(offset),
        }
    }

    pub(crate) fn pop(&mut self) {
        match &mut self.storage {
            OffsetStorage::Compact(offsets) => {
                if offsets.len() > 1 {
                    offsets.pop();
                }
            }
            OffsetStorage::Wide(offsets) => {
                if offsets.len() > 1 {
                    offsets.pop();
                }
            }
        }
    }

    pub(crate) fn len(&self) -> usize {
        match &self.storage {
            OffsetStorage::Compact(offsets) => offsets.len(),
            OffsetStorage::Wide(offsets) => offsets.len(),
        }
    }

    pub(crate) fn get(&self, index: usize) -> Option<usize> {
        match &self.storage {
            OffsetStorage::Compact(offsets) => offsets.get(index).map(|offset| *offset as usize),
            OffsetStorage::Wide(offsets) => offsets.get(index).copied(),
        }
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        match &self.storage {
            OffsetStorage::Compact(offsets) => offsets
                .capacity()
                .saturating_mul(std::mem::size_of::<u32>()),
            OffsetStorage::Wide(offsets) => offsets
                .capacity()
                .saturating_mul(std::mem::size_of::<usize>()),
        }
    }

    #[cfg(test)]
    fn is_compact(&self) -> bool {
        matches!(self.storage, OffsetStorage::Compact(_))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PreviewBuffer {
    text: String,
    line_starts: CompactLineStarts,
    cursor: Cursor,
    presentation_id: u64,
}

impl PreviewBuffer {
    pub(crate) fn from_text(text: &str) -> Self {
        Self::from_owned_text(text.to_owned())
    }

    pub(crate) fn from_owned_text(text: String) -> Self {
        let text = if text.as_bytes().contains(&b'\r') {
            text.replace("\r\n", "\n").replace('\r', "\n")
        } else {
            text
        };
        let mut line_starts = CompactLineStarts::new();
        for (byte, _) in text.match_indices('\n') {
            line_starts.push(byte.saturating_add(1));
        }
        Self::from_parts(text, line_starts)
    }

    pub(crate) fn from_parts(text: String, line_starts: CompactLineStarts) -> Self {
        debug_assert_eq!(line_starts.get(0), Some(0));
        debug_assert!(line_starts
            .get(line_starts.len().saturating_sub(1))
            .is_some_and(|offset| offset <= text.len()));
        Self {
            text,
            line_starts,
            cursor: Cursor::default(),
            presentation_id: next_presentation_id(),
        }
    }

    fn line_bounds(&self, row: usize) -> Option<(usize, usize)> {
        let start = self.line_starts.get(row)?;
        let end = self
            .line_starts
            .get(row.saturating_add(1))
            .map_or(self.text.len(), |next| next.saturating_sub(1));
        Some((start.min(self.text.len()), end.min(self.text.len())))
    }

    fn current_line_len(&self) -> usize {
        self.line_char_count(self.cursor.row).unwrap_or(0)
    }

    fn line_window(&self, row: usize, start_col: usize, width: usize) -> Option<&str> {
        let (start, end) = self.line_bounds(row)?;
        let line = &self.text[start..end];
        let window_start = byte_offset_at_scalar(line, start_col);
        let window_end =
            window_start.saturating_add(byte_offset_at_scalar(&line[window_start..], width));
        Some(&line[window_start..window_end])
    }
}

impl Buffer for PreviewBuffer {
    fn line_count(&self) -> usize {
        self.line_starts.len().max(1)
    }

    fn line(&self, row: usize) -> Option<Cow<'_, str>> {
        let (start, end) = self.line_bounds(row)?;
        Some(Cow::Borrowed(&self.text[start..end]))
    }

    fn visible_lines(&self, start: usize, height: usize) -> Vec<LineView<'_>> {
        let end = start.saturating_add(height).min(self.line_count());
        (start..end)
            .filter_map(|row| {
                self.line_bounds(row)
                    .map(|(line_start, line_end)| LineView {
                        content: Cow::Borrowed(&self.text[line_start..line_end]),
                    })
            })
            .collect()
    }

    fn visible_lines_window(
        &self,
        start: usize,
        height: usize,
        start_col: usize,
        width: usize,
    ) -> Vec<LineView<'_>> {
        let end = start.saturating_add(height).min(self.line_count());
        (start..end)
            .filter_map(|row| {
                self.line_window(row, start_col, width)
                    .map(|content| LineView {
                        content: Cow::Borrowed(content),
                    })
            })
            .collect()
    }

    fn try_visible_lines_window(
        &self,
        start: usize,
        height: usize,
        start_col: usize,
        width: usize,
    ) -> io::Result<Vec<LineView<'_>>> {
        Ok(self.visible_lines_window(start, height, start_col, width))
    }

    fn line_char_count(&self, row: usize) -> Option<usize> {
        self.line(row).map(|line| line.chars().count())
    }

    fn cursor(&self) -> Cursor {
        self.cursor
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn presentation_identity(&self) -> Option<u64> {
        Some(self.presentation_id)
    }

    fn logical_byte_len(&self) -> Option<usize> {
        Some(self.text.len())
    }

    fn set_cursor(&mut self, cursor: Cursor) {
        let row = cursor.row.min(self.line_count().saturating_sub(1));
        let col = cursor.col.min(self.line_char_count(row).unwrap_or(0));
        self.cursor = Cursor { row, col };
    }

    fn to_string(&self) -> String {
        self.text.clone()
    }

    fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(self.text.as_bytes())
    }

    #[cfg(test)]
    fn lines(&self) -> Vec<String> {
        (0..self.line_count())
            .filter_map(|row| self.line(row).map(Cow::into_owned))
            .collect()
    }

    fn insert_char(&mut self, _ch: char) {}

    fn insert_newline(&mut self) {}

    fn delete_back(&mut self) {}

    fn delete_forward(&mut self) {}

    fn move_left(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.cursor.col = self.current_line_len();
        }
    }

    fn move_right(&mut self) {
        if self.cursor.col < self.current_line_len() {
            self.cursor.col += 1;
        } else if self.cursor.row + 1 < self.line_count() {
            self.cursor.row += 1;
            self.cursor.col = 0;
        }
    }

    fn move_up(&mut self) {
        if self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.cursor.col = self.cursor.col.min(self.current_line_len());
        }
    }

    fn move_down(&mut self) {
        if self.cursor.row + 1 < self.line_count() {
            self.cursor.row += 1;
            self.cursor.col = self.cursor.col.min(self.current_line_len());
        }
    }

    fn undo(&mut self) {}

    fn redo(&mut self) {}

    fn edit_history_position(&self) -> u64 {
        0
    }
}

fn next_presentation_id() -> u64 {
    static NEXT_PRESENTATION_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_PRESENTATION_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |identity| {
            (identity < u64::MAX / 2).then(|| identity + 1)
        })
        .expect("preview presentation identity exhausted")
}

fn byte_offset_at_scalar(text: &str, scalar: usize) -> usize {
    text.char_indices()
        .nth(scalar)
        .map_or(text.len(), |(byte, _)| byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn compact_line_starts_promote_without_truncating_overflow() {
        let mut starts = CompactLineStarts::new();
        starts.push(u32::MAX as usize);
        assert!(starts.is_compact());
        let compact_bytes = starts.retained_bytes();

        let overflow = (u32::MAX as usize).saturating_add(1);
        starts.push(overflow);

        assert!(!starts.is_compact());
        assert_eq!(starts.get(1), Some(u32::MAX as usize));
        assert_eq!(starts.get(2), Some(overflow));
        assert!(starts.retained_bytes() > compact_bytes);

        starts.pop();
        assert_eq!(starts.len(), 2);
        assert_eq!(starts.get(1), Some(u32::MAX as usize));
        assert!(!starts.is_compact());
    }

    #[test]
    fn preview_buffer_borrows_rows_windows_and_clamps_navigation() {
        let text = "a\t猫e\u{301}👩\u{200d}💻z\nlast\n".to_string();
        let mut starts = CompactLineStarts::new();
        for (byte, _) in text.match_indices('\n') {
            starts.push(byte + 1);
        }
        let mut buffer = PreviewBuffer::from_parts(text, starts);

        let rows = buffer.visible_lines(0, 2);
        assert!(matches!(rows[0].content, Cow::Borrowed(_)));
        assert_eq!(rows[0].content, "a\t猫e\u{301}👩\u{200d}💻z");
        drop(rows);

        let windows = buffer.visible_lines_window(0, 1, 1, 7);
        assert!(matches!(windows[0].content, Cow::Borrowed(_)));
        assert_eq!(windows[0].content, "\t猫e\u{301}👩\u{200d}💻");
        drop(windows);

        let fallible = buffer
            .try_visible_lines_window(0, 1, 8, usize::MAX)
            .unwrap();
        assert!(matches!(fallible[0].content, Cow::Borrowed("z")));
        drop(fallible);

        let empty = buffer.visible_lines_window(0, 1, usize::MAX, 4);
        assert!(matches!(empty[0].content, Cow::Borrowed("")));
        drop(empty);

        buffer.set_cursor(Cursor { row: 0, col: 99 });
        assert_eq!(buffer.cursor(), Cursor { row: 0, col: 9 });
        buffer.move_down();
        assert_eq!(buffer.cursor(), Cursor { row: 1, col: 4 });
        assert!(buffer.is_read_only());
    }

    #[test]
    fn presentation_identity_survives_clone_and_separates_new_previews() {
        let first = PreviewBuffer::from_parts("first".into(), CompactLineStarts::new());
        let clone = first.clone();
        let second = PreviewBuffer::from_parts("second".into(), CompactLineStarts::new());

        assert_eq!(first.presentation_identity(), clone.presentation_identity());
        assert_ne!(
            first.presentation_identity(),
            second.presentation_identity()
        );
    }
}
