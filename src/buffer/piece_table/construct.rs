//! PieceTable constructors and input normalization.
//!
//! Purpose: keep construction paths for new, borrowed text, and owned open buffers
//!   out of the main piece_table module.
//! Owns: PieceTable::new, PieceTable::from_text, PieceTable::from_owned_text.
//! Must not: perform edits, undo/redo, queries, UI, file I/O, Project, or LLM work.
//! Invariants: CRLF/CR normalize to LF; LF-only owned input moves into original
//!   without cloning; cursor starts at (0,0); initial piece/index/piece_starts are consistent.
//! Phase: 2-ao (open materialization copy-count cleanup).

use crate::buffer::Cursor;

use super::types::{OriginalBacking, Piece, PieceTable, Source};

impl PieceTable {
    pub fn new() -> Self {
        let pieces = vec![Piece {
            source: Source::Original,
            start: 0,
            len: 0,
        }];
        let original = OriginalBacking::empty();
        let index = Self::build_index(&original, "", &pieces);
        let piece_starts = vec![0];
        Self {
            original,
            add: String::new(),
            pieces,
            index,
            cursor: Cursor { row: 0, col: 0 },
            cursor_byte_offset: 0,
            piece_starts,
            undo_stack: crate::buffer::undo::UndoStack::new(),
            recording: true,
        }
    }

    pub fn from_text(text: &str) -> Self {
        let normalized = if text.as_bytes().contains(&b'\r') {
            text.replace("\r\n", "\n").replace('\r', "\n")
        } else {
            text.to_string()
        };
        Self::from_normalized_text(normalized)
    }

    /// Build from an owned read buffer so open paths do not clone large files.
    /// Still normalizes CRLF/CR inputs; LF-only inputs move directly into original.
    pub(crate) fn from_owned_text(text: String) -> Self {
        let normalized = if text.as_bytes().contains(&b'\r') {
            text.replace("\r\n", "\n").replace('\r', "\n")
        } else {
            text
        };
        Self::from_normalized_text(normalized)
    }

    fn from_normalized_text(normalized: String) -> Self {
        let (original, pieces) = if normalized.is_empty() {
            (
                OriginalBacking::empty(),
                vec![Piece {
                    source: Source::Original,
                    start: 0,
                    len: 0,
                }],
            )
        } else {
            let len = normalized.len();
            (
                OriginalBacking::from_owned(normalized),
                vec![Piece {
                    source: Source::Original,
                    start: 0,
                    len,
                }],
            )
        };
        let index = Self::build_index(&original, "", &pieces);
        let piece_starts = vec![0];
        Self {
            original,
            add: String::new(),
            pieces,
            index,
            cursor: Cursor { row: 0, col: 0 },
            cursor_byte_offset: 0,
            piece_starts,
            undo_stack: crate::buffer::undo::UndoStack::new(),
            recording: true,
        }
    }
}
