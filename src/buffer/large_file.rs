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

use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;

use crate::buffer::Cursor;

mod buffer_impl;
pub(crate) mod page_scan;
mod paging;
pub(crate) mod scan;

use scan::scan_utf8_lines;

pub(crate) const SCAN_CHUNK_BYTES: usize = 64 * 1024;
pub(crate) const LINE_CHECKPOINT_INTERVAL_CHARS: usize = 16 * 1024;

/// Read-only file-backed buffer for paged Huge/Extreme-file mode.
pub(crate) struct LargeFileBuffer {
    file: File,
    fd_snapshot: FileMetadataSnapshot,
    #[cfg(test)]
    metadata_check_count: std::cell::Cell<usize>,
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
            #[cfg(test)]
            metadata_check_count: std::cell::Cell::new(0),
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
        #[cfg(test)]
        self.metadata_check_count
            .set(self.metadata_check_count.get() + 1);
        if FileMetadataSnapshot::capture(&self.file)? == self.fd_snapshot {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "large file changed while open",
            ))
        }
    }

    #[cfg(test)]
    fn metadata_check_count(&self) -> usize {
        self.metadata_check_count.get()
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
        Self::bytes_to_string(bytes)
    }

    fn read_range_to_string_unchecked(&self, start: usize, end: usize) -> io::Result<String> {
        let bytes = self.read_range_bytes_unchecked(start, end)?;
        Self::bytes_to_string(bytes)
    }

    fn bytes_to_string(bytes: Vec<u8>) -> io::Result<String> {
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
        self.ensure_fd_unchanged()?;
        self.line_window_to_string_unchecked(row, start_col, width)
    }

    fn line_window_to_string_unchecked(
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
            return self.read_ascii_line_window_unchecked(row, start_col, width);
        }
        self.read_line_window_unchecked(row, start_col, width)
    }

    fn read_ascii_line_window_unchecked(
        &self,
        row: usize,
        start_col: usize,
        width: usize,
    ) -> io::Result<String> {
        let start = self.line_start_byte(row) + start_col;
        let end = start.saturating_add(width).min(self.line_end_byte(row));
        self.read_range_to_string_unchecked(start, end)
    }

    fn read_line_window_unchecked(
        &self,
        row: usize,
        start_col: usize,
        width: usize,
    ) -> io::Result<String> {
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

#[cfg(test)]
#[path = "large_file_tests.rs"]
mod tests;
