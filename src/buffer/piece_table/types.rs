//! Core types for PieceTable (Phase 1B).
//!
//! Source, Piece, LineIndex (data), PieceTable struct definition.
//!
//! Purpose: own the storage model and construction.
//! Owns: original/add buffers, pieces list, line index, cursor state + byte offset cache.
//! Must not: perform heavy UI or project work.
//! Invariants:
//! - Pieces are non-overlapping, cover the logical document, byte ranges respect char boundaries.
//! - LineIndex reflects the logical newlines in the piece concatenation.
//! - cursor_byte_offset always matches the byte position of (cursor.row, cursor.col).
//! Phase: 1B

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

/// Line index over logical byte offsets (rebuild-first in 1B).
#[derive(Clone, Debug, Default)]
pub(crate) struct LineIndex {
    /// Byte offsets of the start of each logical line.
    pub(crate) line_starts: Vec<usize>,
    pub(crate) total_bytes: usize,
}

impl LineIndex {
    pub(crate) fn new() -> Self {
        Self {
            line_starts: vec![0],
            total_bytes: 0,
        }
    }
}

/// PieceTable with original + add + pieces + index + cached cursor offset.
#[derive(Clone, Debug)]
pub struct PieceTable {
    pub(crate) original: String,
    pub(crate) add: String,
    pub(crate) pieces: Vec<Piece>,
    pub(crate) index: LineIndex,
    pub(crate) cursor: Cursor,
    /// Cached global logical byte offset for the cursor.
    /// Avoids full rebuild on every edit for offset calculation.
    pub(crate) cursor_byte_offset: usize,
}
