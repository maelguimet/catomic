//! Purpose: provide immutable descriptor-backed original bytes for PieceTable.
//! Owns: descriptor snapshot checks, positioned range reads, original-line
//!   scalar metadata, checkpoint-assisted cursor mapping, and streamed ranges.
//! Must not: own logical pieces, edits, App policy, rendering, Project, or LLM work.
//! Invariants: ranges are UTF-8 boundaries; line metadata describes the scanned
//!   descriptor; metadata drift fails fallible reads and writes closed.
//! Phase: 2-bj bounded file-backed PieceTable queries.

use std::fs::File;
use std::io::{self, Write};
use std::ops::Range;
use std::os::unix::fs::FileExt;

use crate::buffer::large_file::LineCheckpoint;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileMetadataSnapshot {
    len: u64,
    mtime: Option<std::time::SystemTime>,
}

impl FileMetadataSnapshot {
    pub(crate) fn capture(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            len: metadata.len(),
            mtime: metadata.modified().ok(),
        })
    }
}

#[derive(Debug)]
pub(crate) struct FileOriginalMetadata {
    pub(crate) newline_offsets: Vec<usize>,
    pub(crate) line_char_counts: Vec<usize>,
    pub(crate) line_is_ascii: Vec<bool>,
    pub(crate) line_checkpoints: Vec<LineCheckpoint>,
    pub(crate) line_checkpoint_starts: Vec<usize>,
}

#[derive(Debug)]
pub(crate) struct FileOriginal {
    file: File,
    snapshot: FileMetadataSnapshot,
    metadata: FileOriginalMetadata,
    #[cfg(test)]
    read_bytes: std::sync::atomic::AtomicUsize,
}

impl FileOriginal {
    pub(crate) fn new(
        file: File,
        snapshot: FileMetadataSnapshot,
        metadata: FileOriginalMetadata,
    ) -> Self {
        Self {
            file,
            snapshot,
            metadata,
            #[cfg(test)]
            read_bytes: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn ensure_unchanged(&self) -> io::Result<()> {
        if FileMetadataSnapshot::capture(&self.file)? == self.snapshot {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file-backed original changed while open",
            ))
        }
    }

    fn read_range_unchecked(&self, range: Range<usize>) -> io::Result<Vec<u8>> {
        let mut bytes = vec![0u8; range.len()];
        let mut filled = 0usize;
        while filled < bytes.len() {
            let read = self
                .file
                .read_at(&mut bytes[filled..], (range.start + filled) as u64)?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short read from file-backed original",
                ));
            }
            #[cfg(test)]
            self.read_bytes
                .fetch_add(read, std::sync::atomic::Ordering::Relaxed);
            filled += read;
        }
        Ok(bytes)
    }

    fn read_range(&self, range: Range<usize>) -> io::Result<Vec<u8>> {
        self.ensure_unchanged()?;
        let bytes = self.read_range_unchecked(range)?;
        self.ensure_unchanged()?;
        Ok(bytes)
    }

    pub(crate) fn push_range(&self, range: Range<usize>, out: &mut String) -> io::Result<()> {
        let bytes = self.read_range(range)?;
        out.push_str(as_utf8(&bytes)?);
        Ok(())
    }

    pub(crate) fn write_range(&self, range: Range<usize>, out: &mut dyn Write) -> io::Result<()> {
        self.ensure_unchanged()?;
        let mut offset = range.start;
        while offset < range.end {
            let end = offset.saturating_add(64 * 1024).min(range.end);
            out.write_all(&self.read_range_unchecked(offset..end)?)?;
            offset = end;
        }
        self.ensure_unchanged()
    }

    pub(crate) fn for_each_newline(&self, range: Range<usize>, mut f: impl FnMut(usize)) {
        let start = self
            .metadata
            .newline_offsets
            .partition_point(|offset| *offset < range.start);
        for &offset in &self.metadata.newline_offsets[start..] {
            if offset >= range.end {
                break;
            }
            f(offset);
        }
    }

    pub(crate) fn char_count(&self, range: Range<usize>) -> io::Result<usize> {
        self.ensure_unchanged()?;
        let (row, start_col, end_col) = self.range_columns(&range)?;
        let _ = row;
        self.ensure_unchanged()?;
        Ok(end_col - start_col)
    }

    pub(crate) fn byte_offset_at_char(&self, range: Range<usize>, col: usize) -> io::Result<usize> {
        self.ensure_unchanged()?;
        let (row, start_col, end_col) = self.range_columns(&range)?;
        let target_col = start_col.saturating_add(col).min(end_col);
        let offset = self.byte_offset_at_line_col(row, target_col)?;
        self.ensure_unchanged()?;
        Ok(offset)
    }

    pub(crate) fn push_char_window(
        &self,
        range: Range<usize>,
        skip: usize,
        take: usize,
        out: &mut String,
    ) -> io::Result<usize> {
        if take == 0 || range.is_empty() {
            return Ok(0);
        }
        self.ensure_unchanged()?;
        let (row, start_col, end_col) = self.range_columns(&range)?;
        let window_start_col = start_col.saturating_add(skip).min(end_col);
        let window_end_col = window_start_col.saturating_add(take).min(end_col);
        let start = self.byte_offset_at_line_col(row, window_start_col)?;
        let end = self.byte_offset_at_line_col(row, window_end_col)?;
        let bytes = self.read_range_unchecked(start..end)?;
        out.push_str(as_utf8(&bytes)?);
        self.ensure_unchanged()?;
        Ok(window_end_col - window_start_col)
    }

    fn range_columns(&self, range: &Range<usize>) -> io::Result<(usize, usize, usize)> {
        if range.start > range.end {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reversed range",
            ));
        }
        let row = self.row_for_byte(range.start);
        if range.end > self.line_end(row) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "piece query crosses an original newline",
            ));
        }
        let start_col = self.line_col_at_byte(row, range.start)?;
        let end_col = self.line_col_at_byte(row, range.end)?;
        Ok((row, start_col, end_col))
    }

    fn row_for_byte(&self, byte: usize) -> usize {
        self.metadata
            .newline_offsets
            .partition_point(|newline| *newline < byte)
    }

    fn line_start(&self, row: usize) -> usize {
        row.checked_sub(1)
            .and_then(|previous| self.metadata.newline_offsets.get(previous))
            .map_or(0, |newline| newline + 1)
    }

    fn line_end(&self, row: usize) -> usize {
        self.metadata
            .newline_offsets
            .get(row)
            .copied()
            .unwrap_or(self.snapshot.len as usize)
    }

    fn line_checkpoints(&self, row: usize) -> &[LineCheckpoint] {
        let Some((&start, &end)) = self
            .metadata
            .line_checkpoint_starts
            .get(row)
            .zip(self.metadata.line_checkpoint_starts.get(row + 1))
        else {
            return &[];
        };
        &self.metadata.line_checkpoints[start..end]
    }

    fn line_col_at_byte(&self, row: usize, byte: usize) -> io::Result<usize> {
        if self
            .metadata
            .line_is_ascii
            .get(row)
            .copied()
            .unwrap_or(true)
        {
            return Ok(byte.saturating_sub(self.line_start(row)));
        }
        let checkpoints = self.line_checkpoints(row);
        let idx = checkpoints.partition_point(|checkpoint| checkpoint.byte_offset <= byte);
        let checkpoint = idx.checked_sub(1).map(|i| checkpoints[i]);
        let start = checkpoint.map_or(self.line_start(row), |item| item.byte_offset);
        let col = checkpoint.map_or(0, |item| item.col);
        let bytes = self.read_range_unchecked(start..byte)?;
        Ok(col + as_utf8(&bytes)?.chars().count())
    }

    fn byte_offset_at_line_col(&self, row: usize, col: usize) -> io::Result<usize> {
        let line_chars = self
            .metadata
            .line_char_counts
            .get(row)
            .copied()
            .unwrap_or(0);
        let col = col.min(line_chars);
        if self
            .metadata
            .line_is_ascii
            .get(row)
            .copied()
            .unwrap_or(true)
        {
            return Ok(self.line_start(row) + col);
        }
        let checkpoints = self.line_checkpoints(row);
        let idx = checkpoints.partition_point(|checkpoint| checkpoint.col <= col);
        let checkpoint = idx.checked_sub(1).map(|i| checkpoints[i]);
        let start = checkpoint.map_or(self.line_start(row), |item| item.byte_offset);
        let start_col = checkpoint.map_or(0, |item| item.col);
        let remaining = col - start_col;
        if remaining == 0 {
            return Ok(start);
        }
        let read_end = start
            .saturating_add(remaining.saturating_mul(4))
            .min(self.line_end(row));
        let bytes = self.read_range_unchecked(start..read_end)?;
        let text = utf8_valid_prefix(&bytes)?;
        let relative = text
            .char_indices()
            .nth(remaining)
            .map_or(text.len(), |(offset, _)| offset);
        Ok(start + relative)
    }

    #[cfg(test)]
    pub(crate) fn read_bytes(&self) -> usize {
        self.read_bytes.load(std::sync::atomic::Ordering::Relaxed)
    }
}

fn as_utf8(bytes: &[u8]) -> io::Result<&str> {
    std::str::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn utf8_valid_prefix(bytes: &[u8]) -> io::Result<&str> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(error) if error.error_len().is_none() => as_utf8(&bytes[..error.valid_up_to()]),
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidData, error)),
    }
}
