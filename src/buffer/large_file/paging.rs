//! Purpose: construct and navigate configurable read-only Huge-file pages.
//! Owns: descriptor-stable page scans, page metadata replacement, and cursor reset.
//! Must not: render, decode terminal input, edit content, save, or choose App policy.
//! Invariants: page changes are atomic after a successful stable-descriptor scan;
//!   reverse navigation derives boundaries from the same stable descriptor.
//! Phase: 2-bl configurable paged Huge-file foundation.

use std::fs::File;
use std::io;
use std::path::Path;

use super::page_scan::{find_previous_page_start, scan_utf8_page, PageScan};
use super::{Cursor, FileMetadataSnapshot, LargeFileBuffer};
use crate::buffer::{Buffer, DescriptorPosition, DescriptorSource};

impl LargeFileBuffer {
    pub(crate) fn open_paged(path: impl AsRef<Path>, page_lines: usize) -> io::Result<Self> {
        let file = File::open(path.as_ref())?;
        let fd_snapshot = FileMetadataSnapshot::capture(&file)?;
        let total_bytes = usize::try_from(fd_snapshot.len).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "file size exceeds this platform's addressable range",
            )
        })?;
        let page = scan_utf8_page(&file, 0, page_lines)?;
        if FileMetadataSnapshot::capture(&file)? != fd_snapshot {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "large file changed while scanning page",
            ));
        }
        Ok(Self::from_page_scan(
            file,
            fd_snapshot,
            total_bytes,
            page_lines,
            page,
        ))
    }

    fn from_page_scan(
        file: File,
        fd_snapshot: FileMetadataSnapshot,
        total_bytes: usize,
        page_lines: usize,
        page: PageScan,
    ) -> Self {
        let mut buffer = Self {
            file,
            fd_snapshot,
            #[cfg(test)]
            metadata_check_count: std::cell::Cell::new(0),
            line_starts: Vec::new(),
            line_char_counts: Vec::new(),
            line_is_ascii: Vec::new(),
            line_checkpoints: Vec::new(),
            line_checkpoint_starts: Vec::new(),
            total_bytes,
            page_lines,
            page_number: 1,
            page_start_byte: 0,
            page_end_byte: 0,
            next_page_start: None,
            cursor: Cursor { row: 0, col: 0 },
        };
        buffer.apply_page_scan(page);
        buffer
    }

    pub(super) fn apply_page_scan(&mut self, page: PageScan) {
        self.page_start_byte = page.start_byte;
        self.page_end_byte = page.end_byte;
        self.next_page_start = page.next_page_start;
        self.line_starts = page.lines.line_starts;
        self.line_char_counts = page.lines.line_char_counts;
        self.line_is_ascii = page.lines.line_is_ascii;
        self.line_checkpoints = page.lines.line_checkpoints;
        self.line_checkpoint_starts = page.lines.line_checkpoint_starts;
        self.cursor = Cursor { row: 0, col: 0 };
    }

    pub(super) fn scan_page(&self, start_byte: usize) -> io::Result<PageScan> {
        self.ensure_fd_unchanged()?;
        let page = scan_utf8_page(&self.file, start_byte, self.page_lines)?;
        self.ensure_fd_unchanged()?;
        Ok(page)
    }

    pub(super) fn previous_page_start(&self) -> io::Result<usize> {
        self.ensure_fd_unchanged()?;
        let start = find_previous_page_start(&self.file, self.page_start_byte, self.page_lines)?;
        self.ensure_fd_unchanged()?;
        Ok(start)
    }

    pub(super) fn clone_descriptor_source(&self) -> io::Result<Option<DescriptorSource>> {
        if self.page_lines == usize::MAX {
            return Ok(None);
        }
        self.ensure_fd_unchanged()?;
        Ok(Some(DescriptorSource {
            file: self.file.try_clone()?,
            total_bytes: self.total_bytes as u64,
            page_lines: self.page_lines,
        }))
    }

    pub(super) fn apply_descriptor_position(
        &mut self,
        position: DescriptorPosition,
    ) -> io::Result<bool> {
        let start = usize::try_from(position.page_start).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "search page offset is too large",
            )
        })?;
        let page = self.scan_page(start)?;
        self.page_number = position.page_number;
        self.apply_page_scan(page);
        self.set_cursor(Cursor {
            row: position.row,
            col: position.col,
        });
        Ok(true)
    }
}
