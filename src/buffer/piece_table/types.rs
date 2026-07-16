//! Core types for PieceTable (Phases 1B-1C).
//!
//! Source, Piece, OriginalBacking, LineIndex (data), PieceTable struct definition.
//!
//! Purpose: own the storage model and construction.
//! Owns: original backing/add buffer, pieces list, line index, cursor state + byte offset cache, undo_stack + recording flag.
//! Must not: perform heavy UI or project work.
//! Invariants:
//! - Pieces are non-overlapping, cover the logical document, byte ranges respect char boundaries.
//! - OriginalBacking slices must preserve the same UTF-8 byte-boundary contract as Piece ranges.
//! - LineIndex reflects the logical newlines in the piece concatenation.
//! - cursor_byte_offset always matches the byte position of (cursor.row, cursor.col).
//! Phase: 2-bi file-backed PieceTable foundation.

use std::fs::File;
use std::ops::Range;
use std::os::unix::fs::FileExt;
use std::sync::Arc;
use std::{io, io::Write};

use crate::buffer::line_index::LineIndex;
use crate::buffer::Cursor;

/// Source buffer for a piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Original,
    Add,
}

/// A contiguous byte range in one of the sources.
#[derive(Clone, Debug)]
pub(crate) struct Piece {
    pub(crate) source: Source,
    /// Byte offset into the source String.
    pub(crate) start: usize,
    /// Byte length.
    pub(crate) len: usize,
}

/// Original file/input storage behind Piece ranges.
#[derive(Clone, Debug)]
pub(crate) enum OriginalBacking {
    Owned(String),
    File(Arc<FileOriginal>),
}

#[derive(Debug)]
pub(crate) struct FileOriginal {
    file: File,
    snapshot: FileMetadataSnapshot,
    newline_offsets: Vec<usize>,
}

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

impl FileOriginal {
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

    fn read_range(&self, range: Range<usize>) -> io::Result<Vec<u8>> {
        self.ensure_unchanged()?;
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
            filled += read;
        }
        self.ensure_unchanged()?;
        Ok(bytes)
    }

    fn push_range(&self, range: Range<usize>, out: &mut String) -> io::Result<()> {
        let bytes = self.read_range(range)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        out.push_str(text);
        Ok(())
    }

    fn write_range(&self, range: Range<usize>, out: &mut dyn Write) -> io::Result<()> {
        self.ensure_unchanged()?;
        let mut offset = range.start;
        let mut chunk = vec![0u8; 64 * 1024];
        while offset < range.end {
            let len = chunk.len().min(range.end - offset);
            let mut filled = 0usize;
            while filled < len {
                let read = self
                    .file
                    .read_at(&mut chunk[filled..len], (offset + filled) as u64)?;
                if read == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "short read while streaming file-backed original",
                    ));
                }
                filled += read;
            }
            out.write_all(&chunk[..len])?;
            offset += len;
        }
        self.ensure_unchanged()
    }

    fn for_each_newline(&self, range: Range<usize>, mut f: impl FnMut(usize)) {
        let start = self
            .newline_offsets
            .partition_point(|offset| *offset < range.start);
        for &offset in &self.newline_offsets[start..] {
            if offset >= range.end {
                break;
            }
            f(offset);
        }
    }
}

impl OriginalBacking {
    pub(crate) fn empty() -> Self {
        Self::Owned(String::new())
    }

    pub(crate) fn from_owned(text: String) -> Self {
        Self::Owned(text)
    }

    pub(crate) fn from_file(
        file: File,
        snapshot: FileMetadataSnapshot,
        newline_offsets: Vec<usize>,
    ) -> Self {
        Self::File(Arc::new(FileOriginal {
            file,
            snapshot,
            newline_offsets,
        }))
    }

    pub(crate) fn try_push_slice(&self, range: Range<usize>, out: &mut String) -> io::Result<()> {
        match self {
            Self::Owned(text) => {
                out.push_str(&text[range]);
                Ok(())
            }
            Self::File(file) => file.push_range(range, out),
        }
    }

    pub(crate) fn write_slice(&self, range: Range<usize>, out: &mut dyn Write) -> io::Result<()> {
        match self {
            Self::Owned(text) => out.write_all(text[range].as_bytes()),
            Self::File(file) => file.write_range(range, out),
        }
    }

    pub(crate) fn for_each_newline(&self, range: Range<usize>, mut f: impl FnMut(usize)) {
        match self {
            Self::Owned(text) => {
                for (i, _) in text[range.clone()].match_indices('\n') {
                    f(range.start + i);
                }
            }
            Self::File(file) => file.for_each_newline(range, f),
        }
    }
}

// LineIndex lives in crate::buffer::line_index (single definition, no duplicate).
// PT stores and uses it.

/// PieceTable with original + add + pieces + index + cached cursor offset.
#[derive(Clone, Debug)]
pub struct PieceTable {
    pub(crate) original: OriginalBacking,
    pub(crate) add: String,
    pub(crate) pieces: Vec<Piece>,
    pub(crate) index: LineIndex,
    pub(crate) cursor: Cursor,
    /// Cached global logical byte offset for the cursor.
    /// Avoids full rebuild on every edit for offset calculation.
    pub(crate) cursor_byte_offset: usize,
    /// Prefix sums of piece lengths (parallel to pieces). Enables fast
    /// piece lookup for queries/edits without head scans on every op.
    pub(crate) piece_starts: Vec<usize>,
    /// Undo/redo history (piece deltas only; no full snapshots).
    pub(crate) undo_stack: crate::buffer::undo::UndoStack,
    /// If false, structural edits do not record transactions (suppress during apply).
    pub(crate) recording: bool,
}
