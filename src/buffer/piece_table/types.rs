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
//! Phase: 1B-1C

use std::ops::Range;

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
}

impl OriginalBacking {
    pub(crate) fn empty() -> Self {
        Self::Owned(String::new())
    }

    pub(crate) fn from_owned(text: String) -> Self {
        Self::Owned(text)
    }

    pub(crate) fn push_slice(&self, range: Range<usize>, out: &mut String) {
        match self {
            Self::Owned(text) => out.push_str(&text[range]),
        }
    }

    pub(crate) fn for_each_newline(&self, range: Range<usize>, mut f: impl FnMut(usize)) {
        match self {
            Self::Owned(text) => {
                for (i, _) in text[range.clone()].match_indices('\n') {
                    f(range.start + i);
                }
            }
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
