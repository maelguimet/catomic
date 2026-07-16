//! Purpose: provide a read-only, file-backed Buffer for Huge/Extreme file pages.
//! Owns: file-backed visible-line reads, bounded descriptor streaming, and
//!   read-only movement; delegates initial scanning to the scan submodule.
//! Must not: edit or write back to file content, own App policy, construct watchers,
//!   depend on Project/LLM, or materialize the whole file for rendering/navigation.
//! Invariants: line_starts[0] == 0; per-line metadata lengths match line_starts;
//!   file content was UTF-8 valid at construction; ranged reads use the same
//!   descriptor scanned at open and fail closed if descriptor metadata changes;
//!   cursor row/col stays clamped.
//! Phase: 2-bl configurable paged Huge-file foundation.

use std::borrow::Cow;
use std::fs::File;
use std::io::{self, Write};
use std::os::unix::fs::FileExt;
use std::path::Path;

use crate::buffer::{Buffer, Cursor, LineView, PageInfo};

mod page_scan;
mod paging;
pub(crate) mod scan;

use scan::scan_utf8_lines;

pub(crate) const SCAN_CHUNK_BYTES: usize = 64 * 1024;
pub(crate) const LINE_CHECKPOINT_INTERVAL_CHARS: usize = 16 * 1024;

/// Read-only file-backed buffer for paged Huge/Extreme-file mode.
pub(crate) struct LargeFileBuffer {
    file: File,
    fd_snapshot: FileMetadataSnapshot,
    line_starts: Vec<usize>,
    line_char_counts: Vec<usize>,
    line_is_ascii: Vec<bool>,
    line_checkpoints: Vec<LineCheckpoint>,
    line_checkpoint_starts: Vec<usize>,
    total_bytes: usize,
    page_lines: usize,
    page_number: usize,
    page_start_byte: usize,
    page_end_byte: usize,
    next_page_start: Option<usize>,
    cursor: Cursor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LineCheckpoint {
    pub(crate) col: usize,
    pub(crate) byte_offset: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FileMetadataSnapshot {
    len: u64,
    mtime: Option<std::time::SystemTime>,
}

impl FileMetadataSnapshot {
    pub(super) fn capture(file: &File) -> io::Result<Self> {
        let meta = file.metadata()?;
        Ok(Self {
            len: meta.len(),
            mtime: meta.modified().ok(),
        })
    }
}

impl LargeFileBuffer {
    pub(crate) fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut file = File::open(path.as_ref())?;
        let fd_snapshot = FileMetadataSnapshot::capture(&file)?;
        let scan = scan_utf8_lines(&mut file)?;
        if FileMetadataSnapshot::capture(&file)? != fd_snapshot {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "large file changed while scanning",
            ));
        }
        Ok(Self {
            file,
            fd_snapshot,
            line_starts: scan.line_starts,
            line_char_counts: scan.line_char_counts,
            line_is_ascii: scan.line_is_ascii,
            line_checkpoints: scan.line_checkpoints,
            line_checkpoint_starts: scan.line_checkpoint_starts,
            total_bytes: scan.total_bytes,
            page_lines: usize::MAX,
            page_number: 1,
            page_start_byte: 0,
            page_end_byte: scan.total_bytes,
            next_page_start: None,
            cursor: Cursor { row: 0, col: 0 },
        })
    }

    fn line_start_byte(&self, row: usize) -> usize {
        self.line_starts[row.min(self.line_starts.len().saturating_sub(1))]
    }

    fn line_end_byte(&self, row: usize) -> usize {
        let row = row.min(self.line_starts.len().saturating_sub(1));
        if row + 1 < self.line_starts.len() {
            self.line_starts[row + 1].saturating_sub(1)
        } else if self.next_page_start.is_some() {
            self.page_end_byte.saturating_sub(1)
        } else {
            self.page_end_byte
        }
    }

    fn ensure_fd_unchanged(&self) -> io::Result<()> {
        if FileMetadataSnapshot::capture(&self.file)? == self.fd_snapshot {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "large file changed while open",
            ))
        }
    }

    fn read_range_bytes_unchecked(&self, start: usize, end: usize) -> io::Result<Vec<u8>> {
        if start >= end {
            return Ok(Vec::new());
        }
        let mut out = vec![0u8; end - start];
        let mut filled = 0usize;
        while filled < out.len() {
            let n = self
                .file
                .read_at(&mut out[filled..], (start + filled) as u64)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short read from large file buffer",
                ));
            }
            filled += n;
        }
        Ok(out)
    }

    fn read_range_bytes(&self, start: usize, end: usize) -> io::Result<Vec<u8>> {
        self.ensure_fd_unchanged()?;
        self.read_range_bytes_unchecked(start, end)
    }

    fn read_range_to_string(&self, start: usize, end: usize) -> io::Result<String> {
        let bytes = self.read_range_bytes(start, end)?;
        String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn line_to_string(&self, row: usize) -> io::Result<String> {
        if row >= self.line_starts.len() {
            return Ok(String::new());
        }
        self.read_range_to_string(self.line_start_byte(row), self.line_end_byte(row))
    }

    fn line_window_to_string(
        &self,
        row: usize,
        start_col: usize,
        width: usize,
    ) -> io::Result<String> {
        if row >= self.line_starts.len() || width == 0 {
            return Ok(String::new());
        }
        let line_chars = self.line_char_counts[row];
        if start_col >= line_chars {
            return Ok(String::new());
        }
        if self.line_is_ascii[row] {
            return self.read_ascii_line_window(row, start_col, width);
        }
        self.read_line_window(row, start_col, width)
    }

    fn read_ascii_line_window(
        &self,
        row: usize,
        start_col: usize,
        width: usize,
    ) -> io::Result<String> {
        let start = self.line_start_byte(row) + start_col;
        let end = start.saturating_add(width).min(self.line_end_byte(row));
        self.read_range_to_string(start, end)
    }

    fn read_line_window(&self, row: usize, start_col: usize, width: usize) -> io::Result<String> {
        self.ensure_fd_unchanged()?;
        let checkpoint = self.line_checkpoint_at_or_before(row, start_col);
        let mut pos = checkpoint
            .map(|checkpoint| checkpoint.byte_offset)
            .unwrap_or_else(|| self.line_start_byte(row));
        let end = self.line_end_byte(row);
        let mut carry: Vec<u8> = Vec::new();
        let mut seen = checkpoint.map(|checkpoint| checkpoint.col).unwrap_or(0);
        let mut taken = 0usize;
        let mut out = String::new();

        while pos < end && taken < width {
            let chunk_end = (pos + SCAN_CHUNK_BYTES).min(end);
            let mut bytes = self.read_range_bytes_unchecked(pos, chunk_end)?;
            pos = chunk_end;

            if !carry.is_empty() {
                let mut joined = Vec::with_capacity(carry.len() + bytes.len());
                joined.extend_from_slice(&carry);
                joined.extend_from_slice(&bytes);
                bytes = joined;
                carry.clear();
            }

            let valid_end = match std::str::from_utf8(&bytes) {
                Ok(_) => bytes.len(),
                Err(e) if e.error_len().is_none() => e.valid_up_to(),
                Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            };
            let text = std::str::from_utf8(&bytes[..valid_end])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            for ch in text.chars() {
                if seen >= start_col && taken < width {
                    out.push(ch);
                    taken += 1;
                }
                seen += 1;
                if taken >= width {
                    break;
                }
            }
            if valid_end < bytes.len() {
                carry.extend_from_slice(&bytes[valid_end..]);
            }
        }

        if taken < width && !carry.is_empty() {
            let text = std::str::from_utf8(&carry)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            for ch in text.chars() {
                if seen >= start_col && taken < width {
                    out.push(ch);
                    taken += 1;
                }
                seen += 1;
                if taken >= width {
                    break;
                }
            }
        }

        Ok(out)
    }

    fn line_checkpoints(&self, row: usize) -> &[LineCheckpoint] {
        if row + 1 >= self.line_checkpoint_starts.len() {
            return &[];
        }
        let start = self.line_checkpoint_starts[row];
        let end = self.line_checkpoint_starts[row + 1];
        &self.line_checkpoints[start..end]
    }

    fn line_checkpoint_at_or_before(&self, row: usize, col: usize) -> Option<LineCheckpoint> {
        let checkpoints = self.line_checkpoints(row);
        let idx = checkpoints.partition_point(|checkpoint| checkpoint.col <= col);
        if idx == 0 {
            None
        } else {
            Some(checkpoints[idx - 1])
        }
    }
}

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
        let end = (start + height).min(self.line_count());
        (start..end)
            .map(|row| {
                self.line_window_to_string(row, start_col, width)
                    .map(|content| LineView { content })
            })
            .collect()
    }

    fn line_char_count(&self, row: usize) -> Option<usize> {
        self.line_char_counts.get(row).copied()
    }

    fn cursor(&self) -> Cursor {
        self.cursor
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

#[cfg(test)]
#[path = "large_file_tests.rs"]
mod tests;
