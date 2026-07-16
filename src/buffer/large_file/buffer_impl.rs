//! Purpose: implement the Buffer contract for read-only paged files.
//! Owns: buffer queries, page commands, cursor movement, and descriptor streaming.
//! Must not: open paths, scan page metadata directly, edit content, or choose App policy.
//! Invariants: fallible windows verify descriptor stability before and after their reads;
//!   cursor movement remains clamped to active-page line metadata.
//! Phase: 2-bp Huge-file render probe optimization and size-hygiene split.

use std::borrow::Cow;
use std::io::{self, Write};
use std::os::unix::fs::FileExt;

use super::{LargeFileBuffer, SCAN_CHUNK_BYTES};
use crate::buffer::{Buffer, Cursor, DescriptorPosition, DescriptorSource, LineView, PageInfo};

impl Buffer for LargeFileBuffer {
    fn line_count(&self) -> usize {
        self.line_starts.len().max(1)
    }

    fn line(&self, row: usize) -> Option<Cow<'_, str>> {
        if row >= self.line_starts.len() {
            return None;
        }
        Some(Cow::Owned(self.line_to_string(row).unwrap_or_default()))
    }

    fn visible_lines(&self, start: usize, height: usize) -> Vec<LineView> {
        let end = (start + height).min(self.line_count());
        (start..end)
            .map(|row| LineView {
                content: self.line_to_string(row).unwrap_or_default(),
            })
            .collect()
    }

    fn visible_lines_window(
        &self,
        start: usize,
        height: usize,
        start_col: usize,
        width: usize,
    ) -> Vec<LineView> {
        let end = (start + height).min(self.line_count());
        (start..end)
            .map(|row| LineView {
                content: self
                    .line_window_to_string(row, start_col, width)
                    .unwrap_or_default(),
            })
            .collect()
    }

    fn try_visible_lines_window(
        &self,
        start: usize,
        height: usize,
        start_col: usize,
        width: usize,
    ) -> io::Result<Vec<LineView>> {
        self.ensure_fd_unchanged()?;
        let end = (start + height).min(self.line_count());
        let lines = (start..end)
            .map(|row| {
                self.line_window_to_string_unchecked(row, start_col, width)
                    .map(|content| LineView { content })
            })
            .collect::<io::Result<Vec<_>>>()?;
        self.ensure_fd_unchanged()?;
        Ok(lines)
    }

    fn line_char_count(&self, row: usize) -> Option<usize> {
        self.line_char_counts.get(row).copied()
    }

    fn cursor(&self) -> Cursor {
        self.cursor
    }

    fn set_cursor(&mut self, cursor: Cursor) {
        let row = cursor.row.min(self.line_count().saturating_sub(1));
        let col = cursor
            .col
            .min(self.line_char_counts.get(row).copied().unwrap_or(0));
        self.cursor = Cursor { row, col };
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn page_info(&self) -> Option<PageInfo> {
        if self.page_lines == usize::MAX {
            return None;
        }
        Some(PageInfo {
            page_number: self.page_number,
            start_byte: self.page_start_byte as u64,
            end_byte: self.page_end_byte as u64,
            total_bytes: self.total_bytes as u64,
            has_previous: self.page_start_byte > 0,
            has_next: self.next_page_start.is_some(),
        })
    }

    fn next_page(&mut self) -> io::Result<bool> {
        let Some(start_byte) = self.next_page_start else {
            return Ok(false);
        };
        let page = self.scan_page(start_byte)?;
        self.page_number += 1;
        self.apply_page_scan(page);
        Ok(true)
    }

    fn previous_page(&mut self) -> io::Result<bool> {
        if self.page_start_byte == 0 {
            return Ok(false);
        }
        let start_byte = self.previous_page_start()?;
        let page = self.scan_page(start_byte)?;
        self.page_number = self.page_number.saturating_sub(1).max(1);
        self.apply_page_scan(page);
        Ok(true)
    }

    fn descriptor_source(&self) -> io::Result<Option<DescriptorSource>> {
        self.clone_descriptor_source()
    }

    fn set_descriptor_position(&mut self, position: DescriptorPosition) -> io::Result<bool> {
        self.apply_descriptor_position(position)
    }

    fn to_string(&self) -> String {
        self.read_range_to_string(self.page_start_byte, self.page_end_byte)
            .unwrap_or_default()
    }

    fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        self.ensure_fd_unchanged()?;
        let mut offset = 0usize;
        let mut chunk = vec![0u8; SCAN_CHUNK_BYTES];
        while offset < self.total_bytes {
            let end = offset.saturating_add(chunk.len()).min(self.total_bytes);
            let len = end - offset;
            let mut filled = 0usize;
            while filled < len {
                let read = self
                    .file
                    .read_at(&mut chunk[filled..len], (offset + filled) as u64)?;
                if read == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "short read while streaming large file buffer",
                    ));
                }
                filled += read;
            }
            out.write_all(&chunk[..len])?;
            offset = end;
        }
        self.ensure_fd_unchanged()
    }

    #[cfg(test)]
    fn lines(&self) -> Vec<String> {
        (0..self.line_count())
            .map(|row| self.line_to_string(row).unwrap_or_default())
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
            self.cursor.col = self.line_char_counts[self.cursor.row];
        }
    }

    fn move_right(&mut self) {
        if self.cursor.col < self.line_char_counts[self.cursor.row] {
            self.cursor.col += 1;
        } else if self.cursor.row + 1 < self.line_count() {
            self.cursor.row += 1;
            self.cursor.col = 0;
        }
    }

    fn move_up(&mut self) {
        if self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.cursor.col = self.cursor.col.min(self.line_char_counts[self.cursor.row]);
        }
    }

    fn move_down(&mut self) {
        if self.cursor.row + 1 < self.line_count() {
            self.cursor.row += 1;
            self.cursor.col = self.cursor.col.min(self.line_char_counts[self.cursor.row]);
        }
    }

    fn undo(&mut self) {}

    fn redo(&mut self) {}

    fn edit_history_position(&self) -> u64 {
        0
    }
}
