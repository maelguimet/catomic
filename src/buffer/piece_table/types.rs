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

use std::ops::Range;
use std::sync::Arc;
use std::{io, io::Write};

use crate::buffer::line_index::LineIndex;
use crate::buffer::Cursor;

use super::file_original::{FileMetadataSnapshot, FileOriginal, FileOriginalMetadata};

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

impl OriginalBacking {
    pub(crate) fn empty() -> Self {
        Self::Owned(String::new())
    }

    pub(crate) fn from_owned(text: String) -> Self {
        Self::Owned(text)
    }

    pub(crate) fn from_file(
        file: std::fs::File,
        snapshot: FileMetadataSnapshot,
        metadata: FileOriginalMetadata,
    ) -> Self {
        Self::File(Arc::new(FileOriginal::new(file, snapshot, metadata)))
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

    pub(crate) fn try_char_count(&self, range: Range<usize>) -> io::Result<usize> {
        match self {
            Self::Owned(text) => Ok(text[range].chars().count()),
            Self::File(file) => file.char_count(range),
        }
    }

    pub(crate) fn try_byte_offset_at_char(
        &self,
        range: Range<usize>,
        col: usize,
    ) -> io::Result<usize> {
        match self {
            Self::Owned(text) => Ok(range.start
                + text[range.clone()]
                    .char_indices()
                    .nth(col)
                    .map_or(range.len(), |(offset, _)| offset)),
            Self::File(file) => file.byte_offset_at_char(range, col),
        }
    }

    pub(crate) fn try_push_char_window(
        &self,
        range: Range<usize>,
        skip: usize,
        take: usize,
        out: &mut String,
    ) -> io::Result<usize> {
        match self {
            Self::Owned(text) => {
                let window: String = text[range].chars().skip(skip).take(take).collect();
                let taken = window.chars().count();
                out.push_str(&window);
                Ok(taken)
            }
            Self::File(file) => file.push_char_window(range, skip, take, out),
        }
    }

    #[cfg(test)]
    pub(crate) fn file_read_bytes(&self) -> usize {
        match self {
            Self::Owned(_) => 0,
            Self::File(file) => file.read_bytes(),
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
